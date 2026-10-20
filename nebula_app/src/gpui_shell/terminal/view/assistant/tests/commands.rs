use super::*;
use std::time::Duration;

fn prompt(view: &mut TerminalView, text: &str) {
    feed(view, format!("\r\x1b[2K\x1b]133;A\x07$ \x1b]133;B\x07{text}").as_bytes());
    view.suggest.line_buf = text.into();
    view.command_running = false;
}

fn tick(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(25));
    draw(cx);
}

fn remote_line(view: &mut TerminalView, text: &str) {
    feed(view, format!("\r\x1b[2Kdev@server:/srv$ {text}").as_bytes());
}

fn typed_ssh(view: &mut TerminalView, cx: &mut Context<TerminalView>) {
    view.suggest.suggest_env = crate::display::SuggestEnv::Local;
    prompt(view, "ssh dev@server");
    view.commit_line(cx);
    view.on_command_start(cx);
    view.session.as_ref().unwrap().term.lock().nebula_end_prompt();
    feed(view, b"\r\n");
    remote_line(view, "");
    assert!(matches!(
        view.suggest.suggest_env,
        crate::display::SuggestEnv::Shell { scope: crate::nebula_history::HistoryScope::Ssh(_) }
    ));
    assert!(view.command_running, "the ssh client remains alive");
    assert!(!view.session.as_ref().unwrap().term.lock().nebula_prompt_active());
}

#[gpui::test]
fn ai_command_typed_ssh_without_shell_integration_waits_for_remote_echo(cx: &mut TestAppContext) {
    let _settings = crate::gpui_shell::settings_fixture::lock_theme_studio();
    let (view, mut cx, receiver) = open(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            typed_ssh(view, cx);
            window.focus(&view.focus_handle, cx);
            view.assistant.worker = Some(Arc::new(|_| Ok(advice())));
        })
    });
    cx.simulate_keystrokes("/ a i");
    assert_eq!(input(&receiver), b"/ai");
    view.update(&mut cx, |view, _| {
        remote_line(view, "/ai");
        assert_eq!(
            view.assistant_prompt_input().as_deref(),
            Some("/ai"),
            "mirror={:?}, committed={:?}, env={:?}, program={:?}",
            view.suggest.line_buf,
            view.suggest.last_committed,
            view.suggest.suggest_env,
            view.running_program
        );
    });
    cx.simulate_keystrokes("enter");
    draw(&mut cx);
    assert_eq!(input(&receiver), [0x7f; 3]);
    assert!(
        view.read_with(&cx, |view, cx| view.assistant_command_scope(cx).is_none()),
        "deletion must reach the server before admitting a request"
    );
    view.update(&mut cx, |view, _| remote_line(view, ""));
    cx.simulate_input("Build the project");
    cx.simulate_keystrokes("enter");
    draw(&mut cx);
    assert!(view.read_with(&cx, |view, cx| view.assistant_can_execute(cx)));
    assert_eq!(
        view.read_with(&cx, |view, _| view
            .assistant
            .panel
            .as_ref()
            .unwrap()
            .command_scope
            .as_ref()
            .unwrap()
            .label()),
        "dev@server:/srv$"
    );
    view.update(&mut cx, |view, _| feed(view, b"\x1b[?2004h"));
    click("assistant-execute", &mut cx);
    assert_eq!(input(&receiver), b"\x1b[200~cargo build\x1b[201~");
    view.update(&mut cx, |view, _| feed(view, b"cargo"));
    tick(&mut cx);
    assert!(input(&receiver).is_empty(), "partial remote echo must not submit");
    view.update(&mut cx, |view, _| feed(view, b" build"));
    tick(&mut cx);
    assert_eq!(input(&receiver), b"\r");
    tick(&mut cx);
    assert!(input(&receiver).is_empty());
}

#[gpui::test]
fn ai_command_ssh_insertion_and_confirmation_stay_on_the_reviewed_prompt(cx: &mut TestAppContext) {
    let _settings = crate::gpui_shell::settings_fixture::lock_theme_studio();
    let (view, mut cx, receiver) = open(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            typed_ssh(view, cx);
            view.assistant.worker = Some(Arc::new(|_| Ok(advice())));
            view.open_assistant(AnalysisKind::Explain, Some("cargo buld".into()), window, cx);
        })
    });
    draw(&mut cx);
    click("assistant-analyze", &mut cx);
    assert!(view.read_with(&cx, |view, _| view.assistant_can_insert()));
    click("assistant-insert", &mut cx);
    assert_eq!(input(&receiver), b"cargo build", "ordinary advice only fills the SSH shell");
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            crate::display::nebula_clear_line(&mut view.suggest);
            remote_line(view, "");
            view.open_assistant(
                AnalysisKind::Command,
                Some("Build the project".into()),
                window,
                cx,
            );
            view.send_assistant(window, cx);
        })
    });
    draw(&mut cx);
    view.update(&mut cx, |view, cx| {
        remote_line(view, "draft");
        assert!(!view.assistant_can_execute(cx));
        feed(view, b"\r\x1b[2KPS C:\\work> ");
        assert!(!view.assistant_can_execute(cx), "disconnect cannot redirect to PowerShell");
        remote_line(view, "");
        assert!(view.assistant_can_execute(cx));
    });
    click("assistant-execute", &mut cx);
    assert_eq!(input(&receiver), b"cargo build");
    view.update(&mut cx, |view, _| feed(view, b"\r\x1b[2KPS C:\\work> cargo build"));
    tick(&mut cx);
    assert!(input(&receiver).is_empty(), "echo on a different prompt must never submit");
    assert!(view.read_with(&cx, |view, _| matches!(
        view.assistant.panel.as_ref().unwrap().status,
        Status::CommandUnavailable
    )));
}

#[gpui::test]
fn ai_command_ssh_without_markers_rejects_passwords_running_work_and_tuis(cx: &mut TestAppContext) {
    let _settings = crate::gpui_shell::settings_fixture::lock_theme_studio();
    let (view, mut cx, receiver) = open(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            typed_ssh(view, cx);
            for text in ["Password: /ai", "Continue connecting (yes/no)? /ai", "output /ai"] {
                feed(view, format!("\r\x1b[2K{text}").as_bytes());
                view.suggest.line_buf = "/ai".into();
                assert!(!view.open_ai_command(window, cx), "{text}");
            }
            view.suggest.line_buf.clear();
            remote_line(view, "");
            view.suggest.last_committed = "sleep 30".into();
            assert!(view.assistant_command_scope(cx).is_none());
            view.suggest.last_committed = "ssh dev@server".into();
            assert!(view.assistant_command_scope(cx).is_some());
            feed(view, b"\x1b[?1049h");
            assert!(view.assistant_command_scope(cx).is_none());
            feed(view, b"\x1b[?1049l");
            view.suggest.suggest_env =
                crate::display::SuggestEnv::Ssh { destination: "dev@server".into() };
            view.command_running = false;
            view.running_program = None;
            remote_line(view, "");
            assert!(
                view.assistant_command_scope(cx).is_some(),
                "built-in SSH needs no OSC markers"
            );
            view.command_running = true;
            assert!(
                view.assistant_command_scope(cx).is_none(),
                "active remote work is not a login wrapper"
            );
        })
    });
    assert!(input(&receiver).is_empty());
}

#[gpui::test]
fn ai_command_entry_generation_and_confirmation_use_real_controls(cx: &mut TestAppContext) {
    let _settings = crate::gpui_shell::settings_fixture::lock_theme_studio();
    let (view, mut cx, receiver) = open(cx);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            prompt(view, "");
            window.focus(&view.focus_handle, cx);
            view.assistant.worker = Some(Arc::new(move |request| {
                recorded.lock().unwrap().push(request.clone());
                Ok(advice())
            }));
        })
    });
    cx.simulate_input("/");
    assert_eq!(input(&receiver), b"/");
    assert_eq!(
        view.read_with(&cx, |view, _| view.assistant.panel.as_ref().unwrap().kind),
        AnalysisKind::Chat
    );
    cx.simulate_input("ai");
    assert_eq!(input(&receiver), b"ai");
    view.update(&mut cx, |view, _| prompt(view, "/ai"));
    cx.simulate_keystrokes("enter");
    draw(&mut cx);
    assert_eq!(input(&receiver), [0x7f; 3]);
    assert!(requests.lock().unwrap().is_empty());
    assert_eq!(
        view.read_with(&cx, |view, _| view.assistant.panel.as_ref().unwrap().kind),
        AnalysisKind::Command
    );
    view.update(&mut cx, |view, _| prompt(view, ""));
    cx.simulate_input("Build this Rust project");
    assert_eq!(
        view.read_with(&cx, |view, cx| view
            .assistant
            .panel
            .as_ref()
            .unwrap()
            .input
            .read(cx)
            .value()
            .to_string()),
        "Build this Rust project"
    );
    assert!(
        view.read_with(&cx, |view, cx| view.assistant_command_scope(cx).is_some()),
        "empty prompt must admit generation"
    );
    cx.simulate_keystrokes("enter");
    draw(&mut cx);
    assert!(
        !requests.lock().unwrap().is_empty(),
        "Enter must dispatch generation; state={}",
        view.read_with(&cx, |view, _| match view.assistant.panel.as_ref().unwrap().status {
            Status::Idle => "idle",
            Status::Loading => "loading",
            Status::CommandUnavailable => "unavailable",
            _ => "other",
        })
    );
    assert_eq!(requests.lock().unwrap()[0].kind, AnalysisKind::Command);
    assert_eq!(requests.lock().unwrap()[0].command, "Build this Rust project");
    assert!(input(&receiver).is_empty(), "generating only previews the command");
    assert!(view.read_with(&cx, |view, cx| view.assistant_can_execute(cx)));
    click("assistant-execute", &mut cx);
    assert_eq!(input(&receiver), b"cargo build");
    assert!(!view.read_with(&cx, |view, cx| view.assistant_can_execute(cx)));
    view.update(&mut cx, |view, cx| {
        view.execute_assistant_command(view.assistant.generation, "cargo build", cx);
        feed(view, b"cargo build");
    });
    tick(&mut cx);
    assert_eq!(input(&receiver), b"\r");
    tick(&mut cx);
    assert!(input(&receiver).is_empty());
    assert!(view.read_with(&cx, |view, _| matches!(
        view.assistant.panel.as_ref().unwrap().status,
        Status::Submitted
    )));
}

#[gpui::test]
fn ai_command_paths_escape_and_alternate_screens_keep_normal_input(cx: &mut TestAppContext) {
    let _settings = crate::gpui_shell::settings_fixture::lock_theme_studio();
    let (view, mut cx, receiver) = open(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            for text in ["/", "/ai/bin", "echo /ai", "/a", "/ai "] {
                prompt(view, text);
                assert!(!view.open_ai_command(window, cx), "{text}");
            }
            prompt(view, "/ai");
            feed(view, b"\x1b[?1049h");
            assert!(!view.open_ai_command(window, cx));
            feed(view, b"\x1b[?1049l");
            prompt(view, "/ai");
            assert!(view.open_ai_command(window, cx));
            prompt(view, "");
        })
    });
    input(&receiver);
    cx.simulate_keystrokes("escape");
    draw(&mut cx);
    assert!(input(&receiver).is_empty(), "closing never inserts a slash or Enter");
}

fn ready(view: &Entity<TerminalView>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            prompt(view, "");
            view.assistant.worker = Some(Arc::new(|_| Ok(advice())));
            view.open_assistant(
                AnalysisKind::Command,
                Some("Build the project".into()),
                window,
                cx,
            );
            view.send_assistant(window, cx);
        })
    });
    draw(cx);
}

#[gpui::test]
fn ai_command_changed_context_edit_and_manual_input_revoke_execution(cx: &mut TestAppContext) {
    let _settings = crate::gpui_shell::settings_fixture::lock_theme_studio();
    let (view, mut cx, receiver) = open(cx);
    ready(&view, &mut cx);
    view.update(&mut cx, |view, cx| {
        assert!(view.assistant_can_execute(cx));
        view.cwd = "changed-directory".into();
        view.execute_assistant_command(view.assistant.generation, "cargo build", cx);
        assert!(!view.assistant_can_execute(cx));
    });
    assert!(input(&receiver).is_empty());
    ready(&view, &mut cx);
    cx.simulate_input(" with different options");
    draw(&mut cx);
    assert!(!view.read_with(&cx, |view, cx| view.assistant_can_execute(cx)));
    ready(&view, &mut cx);
    click("assistant-execute", &mut cx);
    assert_eq!(input(&receiver), b"cargo build");
    view.update(&mut cx, |view, cx| {
        view.write_input(b"x".to_vec(), cx);
        feed(view, b"cargo buildx");
    });
    assert_eq!(input(&receiver), b"x");
    tick(&mut cx);
    assert!(input(&receiver).is_empty(), "manual input revokes the delayed Enter");
    ready(&view, &mut cx);
    view.update(&mut cx, |view, cx| {
        let (session, _) = session::test_session();
        view.session = Some(session);
        prompt(view, "");
        assert!(!view.assistant_can_execute(cx));
    });
}

#[gpui::test]
fn ai_command_provider_changes_and_incomplete_echo_never_submit(cx: &mut TestAppContext) {
    let _settings = crate::gpui_shell::settings_fixture::lock_theme_studio();
    let (view, mut cx, receiver) = open(cx);
    ready(&view, &mut cx);
    view.update(&mut cx, |view, cx| {
        crate::ai_providers::save(&crate::ai_providers::load()).unwrap();
        assert!(!view.assistant_can_execute(cx));
        view.execute_assistant_command(view.assistant.generation, "cargo build", cx);
    });
    assert!(input(&receiver).is_empty());
    ready(&view, &mut cx);
    click("assistant-execute", &mut cx);
    assert_eq!(input(&receiver), b"cargo build");
    view.update(&mut cx, |view, _| feed(view, b"cargo"));
    for _ in 0..151 {
        cx.executor().advance_clock(Duration::from_millis(25));
        cx.run_until_parked();
    }
    assert!(input(&receiver).is_empty());
    assert!(view.read_with(&cx, |view, _| matches!(
        view.assistant.panel.as_ref().unwrap().status,
        Status::SubmitFailed
    )));
    ready(&view, &mut cx);
    view.update(&mut cx, |view, cx| {
        view.apply_ssh_stage(crate::ssh_session::SshStage::Ready, cx);
        assert!(!view.assistant_can_execute(cx));
    });
}

#[gpui::test]
fn ai_command_windows_input_protocol_encodes_entry_cleanup_and_confirmation(
    cx: &mut TestAppContext,
) {
    let _settings = crate::gpui_shell::settings_fixture::lock_theme_studio();
    let (view, mut cx, receiver) = open(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            prompt(view, "/ai");
            feed(view, b"\x1b[?9001h");
            let expected = crate::gpui_shell::terminal::keymap::encode(
                &gpui::Keystroke::parse("backspace").unwrap(),
                &view.term_mode(),
            )
            .unwrap();
            assert!(view.open_ai_command(window, cx));
            assert_eq!(input(&receiver), expected.repeat(3));
        })
    });
    ready(&view, &mut cx);
    click("assistant-execute", &mut cx);
    assert_eq!(input(&receiver), b"cargo build");
    let enter = view.read_with(&cx, |view, _| {
        crate::gpui_shell::terminal::keymap::encode(
            &gpui::Keystroke::parse("enter").unwrap(),
            &view.term_mode(),
        )
        .unwrap()
    });
    assert_ne!(enter, b"\r");
    view.update(&mut cx, |view, _| feed(view, b"cargo build"));
    tick(&mut cx);
    assert_eq!(input(&receiver), enter);
}
