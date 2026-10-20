use super::*;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use gpui_component::Root;
use nebula_terminal::event_loop::StreamProcessor;
use std::sync::{Mutex, mpsc::Receiver};

mod queries;

struct AssistantFixture {
    terminal: Entity<TerminalView>,
    panel: Entity<dock::AssistantPanel>,
}

impl Render for AssistantFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().flex().size_full().child(div().flex_1().min_w_0().child(self.terminal.clone())).child(
            div().w(px(400.0)).max_w_full().h_full().flex_shrink_0().child(self.panel.clone()),
        )
    }
}

fn open(cx: &mut TestAppContext) -> (Entity<TerminalView>, VisualTestContext, Receiver<Msg>) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::gpui_shell::math_view::register(cx);
        crate::gpui_shell::terminal::init(cx);
        cx.set_global(Settings::load(nebula_settings::ThemeName::Nord));
    });
    let mut result = None;
    let (_, window) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| {
            TerminalView::new(
                42,
                (80, 24),
                TerminalLaunch::Local {
                    cwd: None,
                    shell: Some(nebula_terminal::tty::Shell::new(
                        "pebrel-test-missing-shell-executable".into(),
                        vec![],
                    )),
                    shell_name: None,
                },
                window,
                cx,
            )
        });
        let receiver = view.update(cx, |view, _| {
            let (session, receiver) = session::test_session();
            view.session = Some(session);
            view.error = None;
            view.exited = None;
            view.ghost_enabled = false;
            view.suggest.suggest_env = crate::display::SuggestEnv::Wsl { distro: "fixture".into() };
            view.assistant.worker = Some(Arc::new(|_| Err(AnalysisError::RequestFailed)));
            receiver
        });
        result = Some((view.clone(), receiver));
        let panel = cx.new(|cx| dock::AssistantPanel::new(&view, window, cx));
        let fixture = cx.new(|_| AssistantFixture { terminal: view, panel });
        Root::new(fixture, window, cx)
    });
    let mut window = window.clone();
    window.simulate_resize(gpui::size(px(900.0), px(700.0)));
    draw(&mut window);
    let (view, receiver) = result.unwrap();
    receiver.try_iter().for_each(drop);
    (view, window, receiver)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let center = cx.debug_bounds(selector).expect("visible assistant control").center();
    cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    draw(cx);
}

fn input(receiver: &Receiver<Msg>) -> Vec<u8> {
    receiver
        .try_iter()
        .filter_map(|m| match m {
            Msg::Input(bytes) => Some(bytes.into_owned()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn feed(view: &mut TerminalView, bytes: &[u8]) {
    let (_, _, _, proxy) = session::test_session_with_events();
    StreamProcessor::default().feed(
        &mut *view.session.as_ref().unwrap().term.lock(),
        &proxy,
        bytes,
    );
}

fn advice() -> AnalysisResult {
    AnalysisResult {
        analysis: "The build subcommand was misspelled.".into(),
        command: Some("cargo build".into()),
        danger: false,
        query: None,
    }
}

#[gpui::test]
fn chat_follow_up_preserves_history_and_draft_across_collapse(cx: &mut TestAppContext) {
    let (view, mut cx, receiver) = open(cx);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.assistant.worker = Some(Arc::new(move |request| {
                recorded.lock().unwrap().push(request.clone());
                Ok(advice())
            }));
            view.open_assistant(AnalysisKind::Chat, None, window, cx);
        })
    });
    draw(&mut cx);
    cx.simulate_input("What does cargo build do?");
    cx.simulate_keystrokes("ctrl-enter");
    draw(&mut cx);
    assert_eq!(requests.lock().unwrap().len(), 1, "Ctrl+Enter must submit through the real input");
    assert_eq!(requests.lock().unwrap()[0].kind, AnalysisKind::Chat);
    assert!(view.read_with(&cx, |view, cx| {
        view.assistant.panel.as_ref().unwrap().input.read(cx).value().is_empty()
    }));
    cx.simulate_input("How do I build a release?");
    click("assistant-close", &mut cx);
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_assistant_default(window, cx)));
    draw(&mut cx);
    click("assistant-analyze", &mut cx);
    {
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].command, "How do I build a release?");
        assert_eq!(requests[1].history.len(), 1);
        assert_eq!(requests[1].history[0].question, "What does cargo build do?");
    }
    click("assistant-new-chat", &mut cx);
    cx.simulate_input("A new question");
    click("assistant-analyze", &mut cx);
    assert!(requests.lock().unwrap()[2].history.is_empty());
    assert!(input(&receiver).is_empty(), "chat must not send any text to the PTY");
}

#[gpui::test]
fn reopening_a_failure_never_relabels_an_unrelated_cached_answer(cx: &mut TestAppContext) {
    let (view, mut cx, receiver) = open(cx);
    cx.update(|window, cx| view.update(cx, |view, cx| {
        feed(view, b"\x1b]133;A\x07$ \x1b]133;B\x07npm missing\r\n\x1b]133;C\x07unknown npm command\r\n\x1b]133;D;1\x07\x1b]133;A\x07$ \x1b]133;B\x07");
        view.assistant.worker = Some(Arc::new(|_| Ok(advice())));
        view.open_assistant(AnalysisKind::Explain, Some("cargo buld".into()), window, cx);
    }));
    draw(&mut cx);
    click("assistant-analyze", &mut cx);
    assert!(view.read_with(&cx, |v, _| matches!(
        v.assistant.panel.as_ref().unwrap().status,
        Status::Ready(_)
    )));
    for _ in 0..2 {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.open_assistant(AnalysisKind::Failure, None, window, cx);
                assert!(
                    matches!(view.assistant.panel.as_ref().unwrap().status, Status::Idle),
                    "an explanation for cargo must not become an npm failure diagnosis"
                );
            })
        });
        draw(&mut cx);
    }
    assert!(input(&receiver).is_empty());
}

#[gpui::test]
fn rendered_assistant_keeps_text_enter_escape_and_restored_focus_out_of_pty(
    cx: &mut TestAppContext,
) {
    let (view, mut cx, receiver) = open(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_assistant(AnalysisKind::Logs, Some("fixture logs".into()), window, cx);
        })
    });
    draw(&mut cx);
    let focus = view.read_with(&cx, |view, cx| view.assistant_focus(cx).unwrap());
    cx.update(|window, cx| window.focus(&focus, cx));
    cx.simulate_input("more logs");
    cx.simulate_keystrokes("enter");
    draw(&mut cx);
    assert!(input(&receiver).is_empty());
    // Opening a manual analysis does not enable monitoring.
    assert!(!view.read_with(&cx, |v, _| v.assistant.auto));
    assert!(view.read_with(&cx, |v, cx| {
        v.assistant.panel.as_ref().unwrap().input.read(cx).value().contains("more logs")
    }));
    cx.simulate_keystrokes("escape");
    draw(&mut cx);
    assert!(
        view.read_with(&cx, |v, _| v.assistant.panel.is_some()),
        "collapsing preserves the draft"
    );
    assert!(input(&receiver).is_empty());
}

#[gpui::test]
fn assistant_pointer_events_do_not_reach_the_terminal_or_steal_input_focus(
    cx: &mut TestAppContext,
) {
    let (view, mut cx, receiver) = open(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            feed(view, b"\x1b[?1003h\x1b[?1006h");
            view.open_assistant(AnalysisKind::Logs, Some("fixture logs".into()), window, cx);
        })
    });
    draw(&mut cx);
    let bounds = cx.debug_bounds("terminal-assistant").unwrap();
    let position = gpui::point(bounds.left() + px(12.0), bounds.top() + px(12.0));
    cx.simulate_mouse_move(position, None, Modifiers::default());
    for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
        cx.simulate_mouse_down(position, button, Modifiers::default());
        cx.simulate_mouse_up(position, button, Modifiers::default());
    }
    cx.simulate_input("more logs");
    draw(&mut cx);
    assert!(input(&receiver).is_empty(), "assistant gestures must not send terminal input/reports");
    assert!(view.read_with(&cx, |v, cx| {
        v.assistant.panel.as_ref().unwrap().input.read(cx).value().contains("more logs")
    }));
}

#[gpui::test]
fn preview_redacts_a_token_cut_by_the_context_limit(cx: &mut TestAppContext) {
    let (view, mut cx, _) = open(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let selection =
                format!("{} {}", "s".repeat(64), ".".repeat(analysis::MAX_CONTEXT_BYTES - 21));
            view.open_assistant(AnalysisKind::Logs, Some(selection), window, cx);
            let value = view.assistant.panel.as_ref().unwrap().input.read(cx).value();
            assert!(value.contains("[redacted]"));
            assert!(!value.contains(&"s".repeat(20)));
        })
    });
}

#[gpui::test]
fn analyze_copy_and_insert_use_real_controls_without_submitting(cx: &mut TestAppContext) {
    let (view, mut cx, receiver) = open(cx);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            feed(view, b"\x1b]133;A\x07$ \x1b]133;B\x07");
            view.assistant.worker = Some(Arc::new(move |request| {
                recorded.lock().unwrap().push(request.clone());
                Ok(advice())
            }));
            view.open_assistant(AnalysisKind::Explain, Some("cargo buld".into()), window, cx);
        })
    });
    draw(&mut cx);
    click("assistant-analyze", &mut cx);
    assert_eq!(requests.lock().unwrap()[0].command, "cargo buld");
    click("assistant-copy", &mut cx);
    assert!(view.read_with(&cx, |v, _| v.assistant.panel.as_ref().unwrap().copied));
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text())),
        Some(advice().analysis)
    );
    assert!(view.read_with(&cx, |v, _| v.assistant_can_insert()));
    click("assistant-insert", &mut cx);
    assert_eq!(input(&receiver), b"cargo build");
    assert!(!view.read_with(&cx, |v, _| v.assistant_can_insert()));
    let focus = view.read_with(&cx, |v, cx| v.assistant_focus(cx).unwrap());
    cx.update(|window, cx| window.focus(&focus, cx));
    cx.simulate_input("changed context");
    draw(&mut cx);
    assert!(
        view.read_with(&cx, |v, _| matches!(
            v.assistant.panel.as_ref().unwrap().status,
            Status::Idle
        ))
    );
}

#[gpui::test]
fn cancel_control_rejects_late_results_and_retry_survives_failure(cx: &mut TestAppContext) {
    let (view, mut cx, receiver) = open(cx);
    let mut pending_generation = 0;
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_assistant(AnalysisKind::Logs, Some("fixture logs".into()), window, cx);
            view.assistant.panel.as_mut().unwrap().status = Status::Loading;
            pending_generation = view.assistant.generation;
            view.toggle_assistant_auto(cx);
            view.toggle_assistant_auto(cx);
            assert_eq!(view.assistant.generation, pending_generation);
            assert!(matches!(view.assistant.panel.as_ref().unwrap().status, Status::Loading));
        })
    });
    draw(&mut cx);
    click("assistant-cancel", &mut cx);
    view.update(&mut cx, |view, cx| {
        view.finish_assistant(pending_generation, 0, false, Ok(advice()), cx);
        assert!(matches!(view.assistant.panel.as_ref().unwrap().status, Status::Idle));
    });
    click("assistant-analyze", &mut cx);
    assert!(view.read_with(&cx, |v, _| matches!(
        v.assistant.panel.as_ref().unwrap().status,
        Status::Error(AnalysisError::RequestFailed)
    )));
    view.update(&mut cx, |view, _| {
        view.assistant.worker = Some(Arc::new(|_| Ok(advice())));
    });
    click("assistant-analyze", &mut cx);
    assert!(view.read_with(&cx, |v, _| matches!(
        v.assistant.panel.as_ref().unwrap().status,
        Status::Ready(_)
    )));
    click("assistant-close", &mut cx);
    assert!(view.read_with(&cx, |v, _| v.assistant.panel.is_some()));
    assert!(input(&receiver).is_empty());
}

#[gpui::test]
fn narrow_panel_and_stale_results_preserve_current_analysis(cx: &mut TestAppContext) {
    let (view, mut cx, receiver) = open(cx);
    cx.simulate_resize(gpui::size(px(300.0), px(400.0)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_assistant(AnalysisKind::Logs, Some("new logs".into()), window, cx);
            let current = view.assistant.generation;
            view.finish_assistant(current.wrapping_sub(1), 0, false, Ok(advice()), cx);
            assert!(matches!(view.assistant.panel.as_ref().unwrap().status, Status::Idle));
            view.finish_assistant(current, 0, false, Err(AnalysisError::RequestFailed), cx);
            assert!(matches!(
                view.assistant.panel.as_ref().unwrap().status,
                Status::Error(AnalysisError::RequestFailed)
            ));
            view.close_assistant(window, cx);
            view.finish_assistant(current, 0, false, Ok(advice()), cx);
            assert!(matches!(view.assistant.panel.as_ref().unwrap().status, Status::Error(_)));
            view.open_assistant(AnalysisKind::Logs, None, window, cx);
        })
    });
    draw(&mut cx);
    let bounds = cx.debug_bounds("terminal-assistant").unwrap();
    assert!(bounds.origin.x >= px(0.0));
    assert!(bounds.right() <= px(300.0));
    assert!(input(&receiver).is_empty());
}

#[gpui::test]
fn failure_capture_excludes_neighbor_commands_and_auto_analysis_never_writes_pty(
    cx: &mut TestAppContext,
) {
    let (view, mut cx, receiver) = open(cx);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    view.update(&mut cx, |view, cx| {
        view.assistant.worker = Some(Arc::new(move |r| { recorded.lock().unwrap().push(r.clone()); Ok(advice()) }));
        feed(view, b"\x1b]133;A\x07$ \x1b]133;B\x07cargo buld\r\n\x1b]133;C\x07unknown subcommand\r\n\x1b]133;D;101\x07\x1b]133;A\x07$ \x1b]133;B\x07");
        let failure = view.assistant_failure_request(cx).unwrap();
        assert_eq!(failure.command, "cargo buld");
        assert_eq!(failure.exit_code, Some(101));
        assert_eq!(failure.output, "unknown subcommand\n");
        view.observe_assistant_failure(cx);
        assert!(requests.lock().unwrap().is_empty());
        view.toggle_assistant_auto(cx);
        // Enabling monitoring observes future completions; it does not upload
        // an earlier failed command retrospectively.
        view.observe_assistant_failure(cx);
        assert!(requests.lock().unwrap().is_empty());
        feed(view, b"cargo buld\r\n\x1b]133;C\x07unknown subcommand\r\n\x1b]133;D;101\x07\x1b]133;A\x07$ \x1b]133;B\x07");
        view.observe_assistant_failure(cx);
        view.observe_assistant_failure(cx);
    });
    draw(&mut cx);
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert!(view.read_with(&cx, |v, _| v.assistant_has_advice()));
    assert!(input(&receiver).is_empty());
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_assistant(AnalysisKind::Failure, None, window, cx);
            assert!(matches!(view.assistant.panel.as_ref().unwrap().status, Status::Ready(_)));
            feed(view, b"echo success\r\n\x1b]133;C\x07neighbor output\r\n\x1b]133;D;0\x07");
            let request = view.assistant_failure_request(cx).unwrap();
            assert!(!request.output.contains("neighbor"));
            view.prompt_input_epoch += 1;
            assert!(!view.assistant_can_insert());
        })
    });
}
