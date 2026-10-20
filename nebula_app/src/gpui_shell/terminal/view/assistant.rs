//! Pane-owned advice, explicit context capture and stale-result isolation.
use super::*;
use crate::ai_assistant::analysis::{
    self, AnalysisError, AnalysisKind, AnalysisRequest, AnalysisResult,
};
use crate::gpui_shell::prelude::InputState;
use nebula_terminal::term::reading::{CompletedCommand, ReadingPoint};

mod chrome;
pub(super) mod dock;
mod query;
#[cfg(all(test, feature = "gpui-test-support"))]
mod tests;

#[derive(Default)]
pub(super) struct Assistant {
    panel: Option<Panel>,
    auto: bool,
    generation: u64,
    task: Option<gpui::Task<()>>,
    automatic_pending: bool,
    last_request: Option<AnalysisRequest>,
    last_outcome: Option<(u64, Result<AnalysisResult, AnalysisError>)>,
    auto_request: Option<AnalysisRequest>,
    auto_outcome: Option<(u64, Result<AnalysisResult, AnalysisError>)>,
    last_completed: Option<(u64, u64)>,
    last_auto: Option<std::time::Instant>,
    history: Vec<analysis::ConversationTurn>,
    scroll: gpui::ScrollHandle,
    query_target: Option<query::Target>,
    query_cancellation: Option<crate::ai_assistant::query::Cancellation>,
    #[cfg(test)]
    query_worker: Option<
        Arc<
            dyn Fn(
                    &crate::ai_assistant::query::Query,
                    Arc<std::sync::atomic::AtomicBool>,
                ) -> Result<String, AnalysisError>
                + Send
                + Sync,
        >,
    >,
    #[cfg(test)]
    worker: Option<
        Arc<dyn Fn(&AnalysisRequest) -> Result<AnalysisResult, AnalysisError> + Send + Sync>,
    >,
}

struct Panel {
    input: gpui::Entity<InputState>,
    _input_events: gpui::Subscription,
    kind: AnalysisKind,
    failure_request: Option<AnalysisRequest>,
    status: Status,
    result_epoch: u64,
    copied: bool,
}

enum Status {
    Idle,
    Loading,
    Querying,
    Ready(AnalysisResult),
    Error(AnalysisError),
    Inserted,
}

impl Assistant {
    pub(super) fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.task = None;
        self.query_cancellation = None;
        self.automatic_pending = false;
        if let Some(panel) = &mut self.panel
            && matches!(panel.status, Status::Loading | Status::Querying)
        {
            panel.status = Status::Idle;
        }
    }
}

impl TerminalView {
    pub(in crate::gpui_shell) fn open_assistant_default(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = &self.assistant.panel {
            window.focus(&panel.input.focus_handle(cx), cx);
            cx.emit(TerminalViewEvent::AssistantVisibility(true));
            cx.notify();
            return;
        }
        let kind = if self.assistant.auto_outcome.is_some() {
            AnalysisKind::Failure
        } else {
            AnalysisKind::Chat
        };
        self.open_assistant(kind, None, window, cx);
    }

    pub(in crate::gpui_shell) fn assistant_has_advice(&self) -> bool {
        self.assistant.auto_outcome.is_some()
    }

    pub(super) fn assistant_focus(&self, cx: &App) -> Option<FocusHandle> {
        self.assistant.panel.as_ref().map(|p| p.input.focus_handle(cx))
    }

    pub(super) fn dismiss_assistant(&mut self) {
        self.assistant.cancel();
    }

    pub(in crate::gpui_shell) fn open_assistant(
        &mut self,
        kind: AnalysisKind,
        selection: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prepare_assistant(kind, selection, window, cx);
        if let Some(panel) = &self.assistant.panel {
            window.focus(&panel.input.focus_handle(cx), cx);
        }
        cx.emit(TerminalViewEvent::AssistantVisibility(true));
        cx.notify();
    }

    fn prepare_assistant(
        &mut self,
        kind: AnalysisKind,
        selection: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.assistant.cancel();
        self.clear_output_hover(cx);
        let failure_request =
            (kind == AnalysisKind::Failure).then(|| self.assistant_failure_request(cx)).flatten();
        let text = match kind {
            AnalysisKind::Chat => selection.unwrap_or_default(),
            AnalysisKind::Explain => selection.unwrap_or_else(|| {
                self.completion_editor
                    .snapshot(self.prompt_input_epoch)
                    .map(|s| s.line.clone())
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| {
                        (!self.suggest.screen_line.trim().is_empty())
                            .then(|| self.suggest.screen_line.clone())
                    })
                    .unwrap_or_else(|| self.suggest.last_committed.clone())
            }),
            AnalysisKind::Logs => selection
                .or_else(|| self.session.as_ref().and_then(|s| s.term.lock().selection_to_string()))
                .unwrap_or_default(),
            AnalysisKind::Failure => {
                failure_request.as_ref().map(|request| request.output.clone()).unwrap_or_default()
            },
        };
        let text = analysis::redacted_tail(&text, analysis::MAX_CONTEXT_BYTES);
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx).multi_line(true).soft_wrap(true);
            input.set_value(text, window, cx);
            input
        });
        let input_events = cx.subscribe_in(
            &input,
            window,
            |view, input, event: &gpui_component::input::InputEvent, window, cx| {
                if matches!(
                    event,
                    gpui_component::input::InputEvent::PressEnter { secondary: true, .. }
                ) && view
                    .assistant
                    .panel
                    .as_ref()
                    .is_some_and(|panel| panel.input.entity_id() == input.entity_id())
                {
                    view.send_assistant(window, cx);
                    return;
                }
                if matches!(event, gpui_component::input::InputEvent::Change)
                    && let Some(panel) = &mut view.assistant.panel
                    && panel.input.entity_id() == input.entity_id()
                    && !matches!(panel.status, Status::Loading | Status::Querying)
                {
                    panel.status = Status::Idle;
                    panel.copied = false;
                    cx.notify();
                }
            },
        );
        let cached = if failure_request.is_some() {
            if self.assistant.auto_request == failure_request {
                self.assistant.auto_outcome.clone()
            } else if self.assistant.last_request == failure_request {
                self.assistant.last_outcome.clone()
            } else {
                None
            }
        } else {
            None
        };
        let (result_epoch, status) =
            cached.map_or((self.prompt_input_epoch, Status::Idle), |(epoch, result)| {
                (
                    epoch,
                    match result {
                        Ok(result) => Status::Ready(result),
                        Err(e) => Status::Error(e),
                    },
                )
            });
        self.assistant.panel = Some(Panel {
            input: input.clone(),
            _input_events: input_events,
            kind,
            failure_request,
            status,
            result_epoch,
            copied: false,
        });
        cx.notify();
    }

    fn assistant_context(&self, kind: AnalysisKind, cx: &App) -> AnalysisRequest {
        let shell = match &self.session_launch {
            crate::session::LaunchSession::Shell { program, .. } => program.as_str(),
            crate::session::LaunchSession::Default => "default shell",
            crate::session::LaunchSession::Profile { name, .. } => name.as_str(),
            crate::session::LaunchSession::Ssh { .. } => "remote shell (OS unknown)",
        };
        AnalysisRequest {
            kind,
            command: String::new(),
            output: String::new(),
            cwd: self.cwd.clone(),
            environment: format!(
                "Client OS: {}; launch shell: {shell}; current scope: {:?}",
                std::env::consts::OS,
                self.suggest.suggest_env
            ),
            exit_code: None,
            language: format!("{:?}", crate::gpui_shell::config::ui_language(cx)),
            history: Vec::new(),
            query_scope: None,
        }
    }

    fn assistant_failure_request(&self, cx: &App) -> Option<AnalysisRequest> {
        let session = self.session.as_ref()?;
        let term = session.term.lock();
        let command =
            term.completed_commands().rev().find(|c| c.exit_code.is_some_and(|code| code != 0))?;
        let output = command_tail(&term, command)?;
        let mut request = self.assistant_context(AnalysisKind::Failure, cx);
        request.command = command.command.clone();
        request.exit_code = command.exit_code;
        request.output = output;
        Some(request)
    }

    fn submit_assistant(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = &self.assistant.panel else { return };
        if matches!(panel.status, Status::Loading | Status::Querying) {
            return;
        }
        let text = panel.input.read(cx).value().to_string();
        let mut request = if panel.kind == AnalysisKind::Failure {
            let Some(request) = panel.failure_request.clone() else {
                self.assistant.panel.as_mut().unwrap().status =
                    Status::Error(AnalysisError::EmptyContext);
                cx.notify();
                return;
            };
            request
        } else {
            self.assistant_context(panel.kind, cx)
        };
        if matches!(panel.kind, AnalysisKind::Explain | AnalysisKind::Chat) {
            request.command = text.trim().to_owned();
        } else {
            request.output = text;
        }
        if panel.kind == AnalysisKind::Chat {
            request.history = self.assistant.history.clone();
        }
        self.start_assistant(request, false, cx);
    }

    fn send_assistant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = &self.assistant.panel else { return };
        if matches!(panel.status, Status::Loading | Status::Querying) {
            return;
        }
        let input = panel.input.clone();
        let chat = panel.kind == AnalysisKind::Chat;
        if chat && input.read(cx).value().trim().is_empty() {
            return;
        }
        self.submit_assistant(cx);
        if chat {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    fn start_assistant(
        &mut self,
        mut request: AnalysisRequest,
        automatic: bool,
        cx: &mut Context<Self>,
    ) {
        self.assistant.cancel();
        self.assistant.query_target =
            if !automatic && matches!(request.kind, AnalysisKind::Chat | AnalysisKind::Logs) {
                self.assistant_query_target()
            } else {
                None
            };
        request.query_scope =
            self.assistant.query_target.as_ref().map(|target| target.label.clone());
        let generation = self.assistant.generation;
        self.assistant.automatic_pending = automatic;
        let input_epoch = self.prompt_input_epoch;
        #[cfg(test)]
        let worker = self.assistant.worker.clone();
        if automatic {
            self.assistant.auto_request = Some(request.clone());
            self.assistant.auto_outcome = None;
            cx.emit(TerminalViewEvent::AssistantChanged);
        } else {
            self.assistant.last_request = Some(request.clone());
            self.assistant.last_outcome = None;
            if let Some(panel) = &mut self.assistant.panel {
                panel.status = Status::Loading;
                panel.copied = false;
            }
        }
        self.assistant.task = Some(cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move {
                    #[cfg(test)]
                    if let Some(worker) = worker {
                        return worker(&request);
                    }
                    analysis::request(&request)
                })
                .await;
            let _ = view.update(cx, |view, cx| {
                view.finish_assistant(generation, input_epoch, automatic, result, cx);
            });
        }));
        self.assistant.scroll.set_offset(gpui::point(px(0.0), px(-1_000_000.0)));
        cx.notify();
    }

    fn finish_assistant(
        &mut self,
        generation: u64,
        input_epoch: u64,
        automatic: bool,
        result: Result<AnalysisResult, AnalysisError>,
        cx: &mut Context<Self>,
    ) {
        if self.assistant.generation != generation || self.exited.is_some() {
            return;
        }
        let request =
            if automatic { &self.assistant.auto_request } else { &self.assistant.last_request };
        if input_epoch == self.prompt_input_epoch
            && request.as_ref().is_some_and(|r| {
                r.cwd == self.cwd && r.environment == self.assistant_context(r.kind, cx).environment
            })
            && let Ok(result) = &result
            && let Some(command) = &result.command
        {
            self.remember_prediction_advice(command);
        }
        self.assistant.task = None;
        self.assistant.automatic_pending = false;
        if automatic {
            self.assistant.auto_outcome = Some((input_epoch, result.clone()));
            cx.emit(TerminalViewEvent::AssistantChanged);
        } else {
            self.assistant.last_outcome = Some((input_epoch, result.clone()));
            if let (Some(request), Ok(result)) = (&self.assistant.last_request, &result) {
                analysis::remember(&mut self.assistant.history, request, result);
            }
        }
        if !automatic && let Some(panel) = &mut self.assistant.panel {
            panel.result_epoch = input_epoch;
            panel.status = match result {
                Ok(result) => Status::Ready(result),
                Err(e) => Status::Error(e),
            };
            self.assistant.scroll.set_offset(gpui::point(px(0.0), px(-1_000_000.0)));
        }
        cx.notify();
    }

    /// Called only on a real OSC command-end event. No screen keywords or
    /// output timers infer failure; unsupported shells keep manual actions.
    pub(super) fn observe_assistant_failure(&mut self, cx: &mut Context<Self>) {
        let latest = self.session.as_ref().and_then(|s| {
            let term = s.term.lock();
            term.completed_commands().next_back().cloned()
        });
        let Some(command) = latest else { return };
        let identity = (command.output.epoch, command.id);
        if self.assistant.last_completed.replace(identity) == Some(identity) {
            return;
        }
        if !self.assistant.auto
            || self.assistant.task.is_some()
            || self.assistant.last_auto.is_some_and(|t| t.elapsed() < crate::ai_assistant::COOLDOWN)
            || !command.exit_code.is_some_and(|code| {
                crate::ai_assistant::should_suggest(
                    code,
                    &command.command,
                    crate::display::extract_program(&command.command).as_deref(),
                    &[],
                )
            })
        {
            return;
        }
        let Some(request) = self.assistant_failure_request(cx) else { return };
        self.assistant.last_auto = Some(std::time::Instant::now());
        // A hidden monitor can prepare a suggestion without moving keyboard focus.
        self.start_assistant(request, true, cx);
    }

    fn toggle_assistant_auto(&mut self, cx: &mut Context<Self>) {
        self.assistant.auto = !self.assistant.auto;
        if !self.assistant.auto && self.assistant.automatic_pending {
            self.assistant.cancel();
        }
        cx.notify();
    }

    fn assistant_can_insert(&self) -> bool {
        let Some(panel) = &self.assistant.panel else { return false };
        if panel.result_epoch != self.prompt_input_epoch
            || self.exited.is_some()
            || self.command_running && !self.command_running_disproved
            || self
                .term_mode()
                .intersects(TermMode::ALT_SCREEN | TermMode::VI | TermMode::KITTY_KEYBOARD_PROTOCOL)
        {
            return false;
        }
        let Some(session) = &self.session else { return false };
        let term = session.term.lock();
        crate::display::nebula_prompt_line_from_raw_grid(
            &term,
            term.grid().cursor.point,
            &self.suggest.line_buf,
            &self.suggest.suggest_env,
        )
        .is_some_and(|line| line.input.trim().is_empty())
    }

    fn insert_assistant_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.assistant_can_insert() {
            return;
        }
        let command = self.assistant.panel.as_ref().and_then(|p| match &p.status {
            Status::Ready(result) => result.command.clone(),
            _ => None,
        });
        let Some(command) = command else { return };
        if !crate::ai_assistant::valid_suggested_command(&command) {
            return;
        }
        self.paste_now_impl(&command, false, cx);
        self.assistant.panel.as_mut().unwrap().status = Status::Inserted;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub(in crate::gpui_shell) fn close_assistant(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_assistant();
        cx.emit(TerminalViewEvent::AssistantVisibility(false));
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }
}

fn command_tail<H: nebula_terminal::event::EventListener>(
    term: &nebula_terminal::Term<H>,
    command: &CompletedCommand,
) -> Option<String> {
    let span = term.command_output_span(command.id).ok()?;
    // Bound the suffix by both rows and cells, including unusually wide grids.
    let width = term.columns().max(1) as u128;
    let end = span.end.line as u128 * width + span.end.column as u128;
    let start = end.saturating_sub(8192);
    let mut cursor = Some(
        ReadingPoint { line: (start / width) as usize, column: (start % width) as usize }
            .max(ReadingPoint { line: span.end.line.saturating_sub(32), column: 0 })
            .max(span.start),
    );
    let mut output = String::new();
    let mut pending = String::new();
    for _ in 0..8 {
        let chunk = term.reading_chunk(span, cursor, 4096, analysis::MAX_CONTEXT_BYTES).ok()?;
        for row in chunk.rows {
            pending.push_str(&row.text);
            if row.hard_break {
                output.push_str(pending.trim_end_matches(' '));
                output.push('\n');
                pending.clear();
            }
        }
        cursor = chunk.next;
        if cursor.is_none() {
            break;
        }
    }
    // Do not present a truncated prefix as the tail if pathological cell data
    // exhausted the bounded reads before reaching the command's end.
    if cursor.is_some() {
        return None;
    }
    output.push_str(pending.trim_end_matches(' '));
    Some(analysis::redacted_tail(&output, analysis::MAX_CONTEXT_BYTES))
}
