//! Bounded non-blocking delivery: preserve semantic order or fail the session explicitly.

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use futures::{Stream, channel::mpsc};
use nebula_terminal::event::Event;

const MAX_EVENTS: usize = 4096;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const BELL_INTERVAL: Duration = Duration::from_millis(100);

type QueuedEvent = (Event, usize);

#[derive(Default)]
struct Budget {
    events: usize,
    bytes: usize,
    failed: bool,
    last_bell: Option<Instant>,
}

#[derive(Clone)]
pub(super) struct EventSender {
    sender: mpsc::UnboundedSender<QueuedEvent>,
    wake_pending: Arc<AtomicBool>,
    bell_pending: Arc<AtomicBool>,
    budget: Arc<Mutex<Budget>>,
    pub(super) native_prompt: NativePromptState,
}

#[derive(Clone, Default)]
pub(super) struct NativePromptState(Arc<Mutex<NativePrompt>>);

/// PTY 接收与用户输入共用的边界；队列消费只唤醒视图，不重新给旧标记盖章。
#[derive(Clone, Copy, Default)]
pub(super) struct NativePrompt {
    pub seen: bool,
    pub input_epoch: u64,
    pub pending: bool,
}

pub(super) struct EventReceiver {
    receiver: mpsc::UnboundedReceiver<QueuedEvent>,
    wake_pending: Arc<AtomicBool>,
    bell_pending: Arc<AtomicBool>,
    budget: Arc<Mutex<Budget>>,
}

pub(super) fn channel() -> (EventSender, EventReceiver) {
    let (sender, receiver) = mpsc::unbounded();
    let wake_pending = Arc::new(AtomicBool::new(false));
    let bell_pending = Arc::new(AtomicBool::new(false));
    let budget = Arc::new(Mutex::new(Budget::default()));
    (
        EventSender {
            sender,
            wake_pending: wake_pending.clone(),
            bell_pending: bell_pending.clone(),
            budget: budget.clone(),
            native_prompt: NativePromptState::default(),
        },
        EventReceiver { receiver, wake_pending, bell_pending, budget },
    )
}

/// Editing a CMD input line does not invalidate a prompt already received from
/// the PTY. Submission and unknown control sequences remain conservative.
pub(super) fn preserves_native_prompt(mut bytes: &[u8]) -> bool {
    let editing_control = |value| {
        matches!(value, 1 | 2 | 5 | 6 | 8 | 9 | 11 | 12 | 14 | 16 | 18 | 21 | 23 | 25 | 27 | 127)
    };
    while let Some((&byte, rest)) = bytes.split_first() {
        if byte != 0x1b {
            if byte.is_ascii_control() && (byte == 27 || !editing_control(u16::from(byte))) {
                return false;
            }
            bytes = rest;
            continue;
        }
        let Some(sequence) = bytes.strip_prefix(b"\x1b[") else { return false };
        let Some(end) = sequence.iter().position(|byte| (0x40..=0x7e).contains(byte)) else {
            return false;
        };
        let parameters = &sequence[..end];
        match sequence[end] {
            b'A'..=b'D' | b'H' | b'F' if parameters.is_empty() => {},
            b'~' if matches!(parameters, b"1" | b"2" | b"3" | b"4" | b"5" | b"6") => {},
            b'_' => {
                let Ok(parameters) = std::str::from_utf8(parameters) else { return false };
                let mut fields = parameters.split(';');
                let mut values = [0u16; 6];
                for value in &mut values {
                    let Some(parsed) = fields.next().and_then(|field| field.parse().ok()) else {
                        return false;
                    };
                    *value = parsed;
                }
                if fields.next().is_some() || values[3] > 1 {
                    return false;
                }
                // Vk;Sc;Uc;Kd;Cs;Rc: Enter is a boundary even if Uc is zero.
                if values[0] == 13
                    || (values[2] < 32 && values[2] != 0 && !editing_control(values[2]))
                {
                    return false;
                }
            },
            _ => return false,
        }
        bytes = &sequence[end + 1..];
    }
    true
}

impl NativePromptState {
    pub(super) fn snapshot(&self) -> NativePrompt {
        *self.0.lock().unwrap()
    }

    pub(super) fn observe_input(&self, input_epoch: u64, preserves_prompt: bool) {
        let mut prompt = self.0.lock().unwrap();
        prompt.input_epoch = input_epoch;
        prompt.pending &= preserves_prompt;
    }

    pub(super) fn consume_native_prompt(&self) {
        self.0.lock().unwrap().pending = false;
    }

    pub(super) fn observe_prompt(&self) {
        let mut prompt = self.0.lock().unwrap();
        prompt.seen = true;
        prompt.pending = true;
    }
}

impl EventSender {
    pub(super) fn send(&self, event: Event) {
        if matches!(&event, Event::UserVar { name, value }
            if name == "pebrel_cmd_prompt" && value == "1")
        {
            self.native_prompt.observe_prompt();
        }
        let pending = match event {
            Event::Wakeup => Some(&self.wake_pending),
            Event::Bell => Some(&self.bell_pending),
            _ => None,
        };
        if pending.is_some_and(|pending| pending.swap(true, Ordering::AcqRel)) {
            return;
        }
        let mut budget = self.budget.lock().unwrap();
        if budget.failed || self.sender.is_closed() {
            return;
        }
        if matches!(event, Event::Bell) {
            if budget.last_bell.is_some_and(|last| last.elapsed() < BELL_INTERVAL) {
                self.bell_pending.store(false, Ordering::Release);
                return;
            }
            budget.last_bell = Some(Instant::now());
        }
        let bytes = event_bytes(&event);
        if budget.events >= MAX_EVENTS || bytes > MAX_BYTES.saturating_sub(budget.bytes) {
            budget.failed = true;
            // Reserve two small control messages beyond the ordinary budget.
            // Blocking here can deadlock: the producer may hold the terminal lock.
            self.enqueue(
                &mut budget,
                Event::PtyFailure("terminal event queue exceeded its resource limit".into()),
            );
            self.enqueue(&mut budget, Event::Exit);
            return;
        }
        self.enqueue(&mut budget, event);
    }

    fn enqueue(&self, budget: &mut Budget, event: Event) {
        let bytes = event_bytes(&event);
        if self.sender.unbounded_send((event, bytes)).is_ok() {
            budget.events += 1;
            budget.bytes += bytes;
        }
    }
}

fn event_bytes(event: &Event) -> usize {
    let payload = match event {
        Event::Title(s)
        | Event::CwdReport(s)
        | Event::Notify(s)
        | Event::PtyWrite(s)
        | Event::PtyFailure(s)
        | Event::ClipboardStore(_, s) => s.capacity(),
        Event::UserVar { name, value } => name.capacity().saturating_add(value.capacity()),
        Event::InlineImage { data, .. } => data.capacity(),
        Event::AiHookEnvelope(data) => data.capacity(),
        _ => 0,
    };
    std::mem::size_of::<QueuedEvent>().saturating_add(payload)
}

impl EventReceiver {
    fn acknowledge(&self, (event, bytes): QueuedEvent) -> Event {
        let mut budget = self.budget.lock().unwrap();
        budget.events -= 1;
        budget.bytes -= bytes;
        if matches!(event, Event::Bell) {
            self.bell_pending.store(false, Ordering::Release);
        }
        if matches!(event, Event::Wakeup) {
            // Clear before processing: output arriving during UI work must
            // still enqueue a subsequent repaint, even on an inactive tab.
            self.wake_pending.store(false, Ordering::Release);
        }
        event
    }

    pub(super) fn try_recv(&mut self) -> Result<Event, mpsc::TryRecvError> {
        let event = self.receiver.try_recv()?;
        Ok(self.acknowledge(event))
    }
}

impl Stream for EventReceiver {
    type Item = Event;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Event>> {
        let result = Pin::new(&mut self.receiver).poll_next(cx);
        result.map(|event| event.map(|event| self.acknowledge(event)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_editor_controls_keep_the_prompt_but_submissions_and_interrupts_end_it() {
        let prompt = NativePromptState::default();
        prompt.observe_prompt();
        for (epoch, bytes) in
            [b"\x15".as_slice(), b"\x17", b"\x1b[85;22;21;1;8;1_"].into_iter().enumerate()
        {
            assert!(preserves_native_prompt(bytes));
            prompt.observe_input(epoch as u64, preserves_native_prompt(bytes));
            assert!(prompt.snapshot().pending);
        }
        for bytes in [b"\r".as_slice(), b"\x03", b"\x1b[13;28;0;1;0;1_"] {
            assert!(!preserves_native_prompt(bytes));
        }
        prompt.observe_input(5, preserves_native_prompt(b"\r"));
        assert!(!prompt.snapshot().pending);
    }

    #[test]
    fn eighty_sessions_keep_all_agent_edges_through_a_repaint_flood() {
        let mut sessions: Vec<_> = (0..80).map(|_| channel()).collect();
        for (id, (sender, _)) in sessions.iter().enumerate() {
            sender.send(Event::CommandStart);
            for _ in 0..1000 {
                sender.send(Event::Wakeup);
            }
            sender.send(Event::AiHookEnvelope(format!("working:{id}").into_bytes()));
            sender.send(Event::AiHookEnvelope(format!("attention:{id}").into_bytes()));
            sender.send(Event::AiHookEnvelope(format!("done:{id}").into_bytes()));
            sender.send(Event::CommandDone { exit_code: Some(0) });
        }
        for (id, (sender, receiver)) in sessions.iter_mut().enumerate() {
            assert!(matches!(receiver.try_recv().unwrap(), Event::CommandStart));
            assert!(matches!(receiver.try_recv().unwrap(), Event::Wakeup));
            for edge in ["working", "attention", "done"] {
                let Event::AiHookEnvelope(bytes) = receiver.try_recv().unwrap() else {
                    panic!("lost agent edge")
                };
                assert_eq!(bytes, format!("{edge}:{id}").into_bytes());
            }
            assert!(matches!(
                receiver.try_recv().unwrap(),
                Event::CommandDone { exit_code: Some(0) }
            ));
            assert!(receiver.try_recv().is_err());
            sender.send(Event::Wakeup);
            assert!(matches!(receiver.try_recv().unwrap(), Event::Wakeup));
        }
    }

    #[test]
    fn sender_clones_share_wakeup_state_but_retain_exit() {
        let (sender, mut receiver) = channel();
        sender.send(Event::Wakeup);
        sender.clone().send(Event::Wakeup);
        sender.send(Event::Exit);
        assert!(matches!(receiver.try_recv().unwrap(), Event::Wakeup));
        assert!(matches!(receiver.try_recv().unwrap(), Event::Exit));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn bell_flood_is_coalesced_and_semantic_events_keep_order() {
        let (sender, mut receiver) = channel();
        sender.send(Event::CommandStart);
        for _ in 0..200_000 {
            sender.send(Event::Bell);
        }
        sender.send(Event::CommandDone { exit_code: Some(1) });
        assert_eq!(sender.budget.lock().unwrap().events, 3);
        assert!(matches!(receiver.try_recv().unwrap(), Event::CommandStart));
        assert!(matches!(receiver.try_recv().unwrap(), Event::Bell));
        assert!(matches!(receiver.try_recv().unwrap(), Event::CommandDone { .. }));
        assert_eq!(sender.budget.lock().unwrap().bytes, 0);
    }

    #[test]
    fn semantic_overflow_is_bounded_and_ends_with_explicit_failure() {
        let (sender, mut receiver) = channel();
        for _ in 0..MAX_EVENTS * 3 {
            sender.send(Event::CommandStart);
        }
        assert_eq!(sender.budget.lock().unwrap().events, MAX_EVENTS + 2);
        for _ in 0..MAX_EVENTS {
            assert!(matches!(receiver.try_recv().unwrap(), Event::CommandStart));
        }
        assert!(matches!(receiver.try_recv().unwrap(), Event::PtyFailure(_)));
        assert!(matches!(receiver.try_recv().unwrap(), Event::Exit));
        assert!(receiver.try_recv().is_err());
        assert_eq!(sender.budget.lock().unwrap().bytes, 0);
    }

    #[test]
    fn payload_capacity_counts_and_oversized_event_never_enters_queue() {
        let (sender, mut receiver) = channel();
        sender.send(Event::AiHookEnvelope(Vec::with_capacity(MAX_BYTES + 1)));
        assert!(sender.budget.lock().unwrap().bytes < 1024);
        assert!(matches!(receiver.try_recv().unwrap(), Event::PtyFailure(_)));
        assert!(matches!(receiver.try_recv().unwrap(), Event::Exit));
        drop(sender);
        assert!(futures::executor::block_on(futures::StreamExt::next(&mut receiver)).is_none());
    }
}
