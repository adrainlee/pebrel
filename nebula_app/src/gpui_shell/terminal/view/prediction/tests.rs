use super::*;
use gpui::{Entity, TestAppContext, VisualTestContext};
use std::sync::{Mutex, mpsc::Receiver};
use std::time::Duration;

fn open(cx: &mut TestAppContext) -> (Entity<TerminalView>, &mut VisualTestContext, Receiver<Msg>) {
    let (view, window, receiver) = super::super::startup_tests::open(cx);
    view.update(window, |view, _| {
        view.ghost_enabled = true;
        view.completion_style = CompletionStyle::Hybrid;
        view.session.as_ref().unwrap().term.lock().nebula_add_prompt_mark();
        view.suggest.cwd = "/prediction-test".into();
        view.prediction.worker = Some(Arc::new(|_| Some("git status --short".into())));
    });
    receiver.try_iter().for_each(drop);
    (view, window, receiver)
}

fn line(view: &Entity<TerminalView>, window: &mut VisualTestContext, text: &str) {
    view.update(window, |view, cx| {
        view.prediction_input();
        view.prompt_input_epoch += 1;
        view.suggest.screen_line = text.into();
        view.refresh_prediction(text, text.len(), cx);
    });
    window.run_until_parked();
}

fn advance(window: &mut VisualTestContext, duration: Duration) {
    window.executor().advance_clock(duration);
    window.run_until_parked();
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

#[gpui::test]
fn waits_for_word_and_debounce_uses_scoped_history_then_tab_fills_without_running(
    cx: &mut TestAppContext,
) {
    let _settings = crate::gpui_shell::settings_fixture::lock_theme_studio();
    let (view, window, receiver) = open(cx);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    view.update(window, |view, _| {
        let scope = crate::nebula_history::HistoryScope::Ssh("prediction-history".into());
        view.suggest.suggest_env = crate::display::SuggestEnv::Shell { scope: scope.clone() };
        crate::completion::record_command(&scope, "git status --short", "/prediction-test");
        crate::completion::record_command(
            &crate::nebula_history::HistoryScope::Local,
            "git private-host-command",
            "C:/host",
        );
        view.remember_prediction_advice("git status --short");
        view.prediction.worker = Some(Arc::new(move |request| {
            recorded.lock().unwrap().push(request.clone());
            Some("git status --short".into())
        }));
    });
    for text in ["g", "gi", "git"] {
        line(&view, window, text);
        advance(window, Duration::from_secs(1));
    }
    assert!(requests.lock().unwrap().is_empty());
    line(&view, window, "git ");
    advance(window, Duration::from_millis(599));
    assert!(requests.lock().unwrap().is_empty());
    advance(window, Duration::from_millis(1));
    {
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].history.contains(&"git status --short".into()));
        assert!(!requests[0].history.iter().any(|h| h.contains("private-host")));
        assert_eq!(requests[0].advice.as_deref(), Some("git status --short"));
    }
    assert!(input(&receiver).is_empty());
    window.update(|window, cx| {
        view.update(cx, |view, cx| {
            assert_eq!(view.suggest.suggestion, "status --short");
            view.on_terminal_tab(&TerminalTab, window, cx);
        })
    });
    assert_eq!(input(&receiver), b"status --short");
    window.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.on_key_down(
                &KeyDownEvent {
                    keystroke: gpui::Keystroke::parse("enter").unwrap(),
                    is_held: false,
                    prefer_character_input: false,
                },
                window,
                cx,
            );
        })
    });
    assert_eq!(input(&receiver), b"\r");
}

#[gpui::test]
fn typing_restarts_debounce_cached_suffixes_do_not_request_again_and_escape_dismisses(
    cx: &mut TestAppContext,
) {
    let _settings = crate::gpui_shell::settings_fixture::lock_theme_studio();
    let (view, window, _) = open(cx);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = calls.clone();
    view.update(window, |view, _| {
        view.prediction.worker = Some(Arc::new(move |_| {
            count.fetch_add(1, Ordering::Relaxed);
            Some("git status --short".into())
        }))
    });
    line(&view, window, "git ");
    advance(window, Duration::from_millis(500));
    line(&view, window, "git st");
    advance(window, Duration::from_millis(500));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    advance(window, Duration::from_millis(100));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    line(&view, window, "git sta");
    advance(window, Duration::from_secs(5));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    view.update(window, |view, cx| {
        assert_eq!(view.suggest.suggestion, "tus --short");
        assert!(view.handle_completion_key("escape", cx));
        view.refresh_prediction("git sta", 7, cx);
        assert!(view.prediction.displayed.is_none());
        assert!(view.prediction.pending.is_none());
    });
    line(&view, window, "git branch ");
    view.update(window, |view, cx| {
        assert!(view.prediction.pending.is_some());
        view.suggest.request_completion_popup();
        assert!(view.handle_completion_key("escape", cx));
        assert!(!view.suggest.completion_popup_requested);
        assert!(view.prediction.pending.is_none());
    });
}

#[gpui::test]
fn delayed_results_and_cached_items_cannot_cross_input_directory_session_or_settings(
    cx: &mut TestAppContext,
) {
    let _settings = crate::gpui_shell::settings_fixture::lock_theme_studio();
    let (view, window, receiver) = open(cx);
    line(&view, window, "git ");
    let old = view.read_with(window, |view, _| view.prediction.identity.clone().unwrap());
    view.update(window, |view, cx| {
        view.prompt_input_epoch += 1;
        view.finish_prediction(&old, Some("git status".into()), cx);
        assert!(view.prediction.cached.is_none());
        view.prompt_input_epoch = old.epoch;
        view.suggest.cwd = "/elsewhere".into();
        view.finish_prediction(&old, Some("git status".into()), cx);
        assert!(view.prediction.cached.is_none());
        view.suggest.cwd = "/prediction-test".into();
        view.finish_prediction(&old, Some("git status".into()), cx);
        let item = view.suggest.suggestion_edit.clone().unwrap();
        let (session, _) = session::test_session();
        session.term.lock().nebula_add_prompt_mark();
        view.session = Some(session);
        assert!(!view.prediction_item_valid(&item));
        view.finish_prediction(&old, Some("git stash".into()), cx);
        assert_eq!(view.prediction.cached.as_ref().unwrap().0, "git status");
        // Same text in a new native editor revision is also a different input.
        view.refresh_prediction("git ", 4, cx);
        let current = view.prediction.identity.clone().unwrap();
        view.completion_editor.invalidate();
        assert!(!view.prediction_matches(&current));
        view.refresh_prediction("git ", 4, cx);
        let current = view.prediction.identity.clone().unwrap();
        view.finish_prediction(&current, Some("git status".into()), cx);
        let item = view.suggest.suggestion_edit.clone().unwrap();
        crate::ai_providers::save(&crate::ai_providers::load()).unwrap();
        assert!(!view.prediction_item_valid(&item));
        view.finish_prediction(&current, Some("git stash".into()), cx);
        assert_eq!(view.prediction.cached.as_ref().unwrap().0, "git status");
        view.apply_ssh_stage(crate::ssh_session::SshStage::Ready, cx);
        assert!(view.prediction.cached.is_none());
        assert!(view.prediction.pending.is_none());
        assert!(view.prediction.identity.is_none());
    });
    assert!(input(&receiver).is_empty());
}

#[gpui::test]
fn alternate_screens_midline_edits_and_disabled_suggestions_never_start_prediction(
    cx: &mut TestAppContext,
) {
    let _settings = crate::gpui_shell::settings_fixture::lock_theme_studio();
    let (view, window, _) = open(cx);
    view.update(window, |view, cx| {
        view.suggest.screen_line = "git status".into();
        view.refresh_prediction("git status", 4, cx);
        assert!(view.prediction.pending.is_none());
        view.ghost_enabled = false;
        view.refresh_prediction("git status", 10, cx);
        assert!(view.prediction.pending.is_none());
        view.ghost_enabled = true;
        super::super::startup_tests::feed(view, b"\x1b[?1049h");
        view.refresh_prediction("git status", 10, cx);
        assert!(view.prediction.pending.is_none());
    });
}

#[gpui::test]
fn real_terminal_paint_and_tab_dispatch_accept_ai_without_submitting(cx: &mut TestAppContext) {
    let _settings = crate::gpui_shell::settings_fixture::lock_theme_studio();
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::gpui_shell::terminal::init(cx);
        crate::gpui_shell::scientific_render::init(cx);
        cx.set_global(Settings::load(nebula_settings::ThemeName::Nord));
    });
    let mut fixture = None;
    let (_, window) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| {
            TerminalView::new(
                99,
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
        let receiver = entity.update(cx, |view, _| {
            let receiver = view.install_completion_test_session();
            view.ghost_enabled = true;
            view.completion_style = CompletionStyle::Hybrid;
            view.prediction.worker = Some(Arc::new(|_| Some("git status --short".into())));
            let (_, _, _, proxy) = session::test_session_with_events();
            nebula_terminal::event_loop::StreamProcessor::default().feed(
                &mut *view.session.as_ref().unwrap().term.lock(),
                &proxy,
                b"\x1b]133;A\x07$ \x1b]133;B\x07git ",
            );
            receiver
        });
        let focus = entity.read(cx).focus_handle.clone();
        window.focus(&focus, cx);
        fixture = Some((entity.clone(), receiver));
        gpui_component::Root::new(entity, window, cx)
    });
    let (view, receiver) = fixture.unwrap();
    window.simulate_resize(gpui::size(px(800.0), px(500.0)));
    window.run_until_parked();
    window.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    window.run_until_parked();
    advance(window, model::DEBOUNCE);
    window.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert_eq!(view.read_with(window, |view, _| view.suggest.screen_line.clone()), "git ");
    assert_eq!(view.read_with(window, |view, _| view.suggest.suggestion.clone()), "status --short");
    receiver.try_iter().for_each(drop);
    window.simulate_keystrokes("tab");
    window.run_until_parked();
    assert_eq!(input(&receiver), b"status --short");
    // The model/Tab must never generate a submission; only the user's Enter does.
    window.simulate_keystrokes("enter");
    window.run_until_parked();
    assert_eq!(input(&receiver), b"\r");
}
