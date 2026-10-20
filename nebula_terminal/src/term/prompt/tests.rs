use crate::event::VoidListener;
use crate::event_loop::StreamProcessor;
use crate::index::{Column, Line};
use crate::term::test::TermSize;
use crate::term::{Config, Term, TermMode};

const APPLICATION_MODES: &[u8] = b"\x1b[?1003h\x1b[?1006h\x1b[>7u";
const PROMPT: &[u8] = b"\x1b]133;A\x07";

fn terminal() -> Term<VoidListener> {
    Term::new(
        Config { kitty_keyboard: true, ..Config::default() },
        &TermSize::new(80, 24),
        VoidListener,
    )
}

fn feed(term: &mut Term<VoidListener>, stream: &mut StreamProcessor, bytes: &[u8]) {
    stream.feed(term, &VoidListener, bytes);
}

fn application_modes() -> TermMode {
    TermMode::MOUSE_MODE
        | TermMode::SGR_MOUSE
        | TermMode::UTF8_MOUSE
        | TermMode::KITTY_KEYBOARD_PROTOCOL
}

#[test]
fn prompt_recovers_abandoned_ssh_input_modes_at_every_chunk_boundary() {
    let wire = [APPLICATION_MODES, b"\r\nConnection closed\r\n", PROMPT, b"$ "].concat();
    for split in 0..=wire.len() {
        let mut term = terminal();
        let mut stream = StreamProcessor::default();
        feed(&mut term, &mut stream, &wire[..split]);
        feed(&mut term, &mut stream, &wire[split..]);
        assert!(!term.mode().intersects(application_modes()), "split={split}");
        assert!(term.nebula_prompt_active());
        assert_eq!(term.grid()[Line(1)][Column(0)].c, 'C');
        assert_eq!(term.grid()[Line(2)][Column(0)].c, '$');
    }
}

#[test]
fn prompt_clears_both_keyboard_stacks_without_changing_native_or_paste_modes() {
    let mut term = terminal();
    let mut stream = StreamProcessor::default();
    feed(&mut term, &mut stream, b"\x1b[?9001h\x1b[?2004h\x1b[>1u");
    feed(&mut term, &mut stream, b"\x1b[?1049h\x1b[>7u\x1b[?1049l");
    feed(&mut term, &mut stream, b"\x1b[?1002h\x1b[?1005h");
    feed(&mut term, &mut stream, PROMPT);
    let preserved = TermMode::WIN32_INPUT_MODE | TermMode::BRACKETED_PASTE;
    assert!(term.mode().contains(preserved));
    assert!(!term.mode().intersects(application_modes()));
    for bytes in [b"\x1b[?1049h".as_slice(), b"\x1b[<u", b"\x1b[?1049l", b"\x1b[<u"] {
        feed(&mut term, &mut stream, bytes);
        assert!(!term.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL));
    }
}

#[test]
fn prompt_does_not_reset_an_alternate_screen_application() {
    let mut term = terminal();
    let mut stream = StreamProcessor::default();
    feed(&mut term, &mut stream, b"\x1b[?1049h");
    feed(&mut term, &mut stream, APPLICATION_MODES);
    let before = *term.mode();
    feed(&mut term, &mut stream, PROMPT);
    assert_eq!(*term.mode(), before);
    assert!(!term.nebula_prompt_active());
}

#[test]
fn silence_plain_text_and_command_done_do_not_reset_negotiated_modes() {
    let mut term = terminal();
    let mut stream = StreamProcessor::default();
    feed(&mut term, &mut stream, APPLICATION_MODES);
    let before = *term.mode();
    for bytes in [b"".as_slice(), b"user@host:~$ ", b"\x1b]133;D;255\x07"] {
        feed(&mut term, &mut stream, bytes);
        assert_eq!(*term.mode(), before);
    }
}

#[test]
fn prompt_recovers_modes_inside_unfinished_synchronized_output() {
    // An interrupted application can leave both synchronized output and input
    // negotiation open. Buffered mode changes must not reappear after recovery.
    let wire = [b"\x1b[?2026h".as_slice(), APPLICATION_MODES, PROMPT, b"$ "].concat();
    for split in 0..=wire.len() {
        let mut term = terminal();
        let mut stream = StreamProcessor::default();
        feed(&mut term, &mut stream, &wire[..split]);
        feed(&mut term, &mut stream, &wire[split..]);
        assert_eq!(stream.sync_bytes_count(), 0, "split={split}");
        assert!(!term.mode().intersects(application_modes()), "split={split}");
        stream.stop_sync(&mut term);
        assert!(!term.mode().intersects(application_modes()));
    }
}

#[test]
fn prompt_allows_fresh_negotiation_in_wire_order() {
    let wire = [APPLICATION_MODES, PROMPT, b"\x1b[>1u\x1b[?1000h\x1b[?1006h"].concat();
    for split in 0..=wire.len() {
        let mut term = terminal();
        let mut stream = StreamProcessor::default();
        feed(&mut term, &mut stream, &wire[..split]);
        feed(&mut term, &mut stream, &wire[split..]);
        assert_eq!(
            *term.mode() & application_modes(),
            TermMode::DISAMBIGUATE_ESC_CODES | TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE,
            "split={split}"
        );
        feed(&mut term, &mut stream, b"\x1b[<u");
        assert!(!term.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL));
    }
}
