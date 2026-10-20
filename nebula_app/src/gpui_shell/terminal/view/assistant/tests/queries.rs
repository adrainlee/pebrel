use super::*;
use crate::ai_assistant::query::{Cancellation, Query};
use std::sync::atomic::{AtomicUsize, Ordering};

fn proposal() -> AnalysisResult {
    AnalysisResult {
        analysis: "Read the application log.".into(),
        command: Some("reboot".into()),
        danger: true,
        query: Some(Query::FileTail {
            path: if cfg!(windows) { "C:/logs/app.log" } else { "/var/log/app.log" }.into(),
        }),
    }
}

fn prepare(view: &mut TerminalView) {
    view.suggest.suggest_env = crate::display::SuggestEnv::Local;
    view.cwd = std::env::current_dir().unwrap().to_string_lossy().into_owned();
    view.exec_context =
        Some(crate::runtime_exec::PaneExecContext::from_pty_options(&Default::default()));
}

#[gpui::test]
fn real_approval_button_runs_one_read_and_returns_evidence_without_touching_pty(
    cx: &mut TestAppContext,
) {
    let (view, mut cx, receiver) = open(cx);
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            prepare(view);
            view.assistant.worker = Some(Arc::new(move |request| {
                recorded.lock().unwrap().push(request.clone());
                Ok(proposal())
            }));
            view.assistant.query_worker = Some(Arc::new(move |query, _| {
                assert_eq!(query, proposal().query.as_ref().unwrap());
                calls.fetch_add(1, Ordering::SeqCst);
                Ok("password=demo-query-secret\nconnection refused".into())
            }));
            view.open_assistant(
                AnalysisKind::Chat,
                Some("Find the cause of the outage".into()),
                window,
                cx,
            );
        })
    });
    draw(&mut cx);
    click("assistant-analyze", &mut cx);
    assert_eq!(count.load(Ordering::SeqCst), 0, "proposing a query is not permission");
    let old_generation = view.read_with(&cx, |view, _| view.assistant.generation);
    click("assistant-approve-query", &mut cx);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    {
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].query_scope.is_some());
        assert!(requests[1].output.contains("connection refused"));
        assert!(!requests[1].output.contains("demo-query-secret"));
        assert_eq!(requests[1].command, "Find the cause of the outage");
    }
    view.update(&mut cx, |view, cx| {
        view.authorize_assistant_query(old_generation, proposal().query.unwrap(), cx)
    });
    draw(&mut cx);
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "stale or duplicate clicks cannot authorize a subsequent query"
    );
    click("assistant-approve-query", &mut cx);
    assert_eq!(count.load(Ordering::SeqCst), 2, "a fresh query needs a fresh click");
    assert!(
        input(&receiver).is_empty(),
        "neither query nor suggested reboot may enter the interactive shell"
    );
}

#[gpui::test]
fn replaced_session_disables_old_authorization_and_typed_ssh_cannot_fall_back_to_local(
    cx: &mut TestAppContext,
) {
    let (view, mut cx, _) = open(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            prepare(view);
            view.assistant.worker = Some(Arc::new(|_| Ok(proposal())));
            view.assistant.query_worker = Some(Arc::new(|_, _| panic!("stale query must not run")));
            view.open_assistant(AnalysisKind::Chat, Some("Read logs".into()), window, cx);
        })
    });
    draw(&mut cx);
    click("assistant-analyze", &mut cx);
    view.update(&mut cx, |view, _| {
        let (replacement, _) = session::test_session();
        view.session = Some(replacement);
        assert!(!view.assistant_can_query(proposal().query.as_ref().unwrap()));
    });
    draw(&mut cx);
    click("assistant-approve-query", &mut cx);
    view.update(&mut cx, |view, _| {
        view.suggest.suggest_env =
            crate::display::SuggestEnv::Ssh { destination: "another@host".into() };
        assert!(
            view.assistant_query_target().is_none(),
            "unverified remote must never read local paths"
        );
    });
}

#[gpui::test]
fn cancel_and_collapse_revoke_in_flight_query_ownership(cx: &mut TestAppContext) {
    let (view, mut cx, receiver) = open(cx);
    for button in ["assistant-cancel", "assistant-close"] {
        let flag = cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.open_assistant(AnalysisKind::Chat, None, window, cx);
                let cancellation = Cancellation::default();
                let flag = cancellation.0.clone();
                view.assistant.query_cancellation = Some(cancellation);
                view.assistant.panel.as_mut().unwrap().status = Status::Querying;
                flag
            })
        });
        draw(&mut cx);
        click(button, &mut cx);
        assert!(flag.load(Ordering::Acquire));
        assert!(view.read_with(&cx, |view, _| matches!(
            view.assistant.panel.as_ref().unwrap().status,
            Status::Idle
        )));
    }
    assert!(input(&receiver).is_empty());
}

#[gpui::test]
fn query_failure_stops_the_model_loop_and_retry_only_requests_a_new_proposal(
    cx: &mut TestAppContext,
) {
    let (view, mut cx, _) = open(cx);
    let queries = Arc::new(AtomicUsize::new(0));
    let calls = queries.clone();
    let models = Arc::new(AtomicUsize::new(0));
    let requests = models.clone();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            prepare(view);
            view.assistant.worker = Some(Arc::new(move |_| {
                requests.fetch_add(1, Ordering::SeqCst);
                Ok(proposal())
            }));
            view.assistant.query_worker = Some(Arc::new(move |_, _| {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(AnalysisError::QueryFailed)
            }));
            view.open_assistant(AnalysisKind::Chat, Some("Read logs".into()), window, cx);
        })
    });
    draw(&mut cx);
    click("assistant-analyze", &mut cx);
    click("assistant-approve-query", &mut cx);
    assert_eq!(queries.load(Ordering::SeqCst), 1);
    assert_eq!(models.load(Ordering::SeqCst), 1);
    assert!(view.read_with(&cx, |view, _| matches!(
        view.assistant.panel.as_ref().unwrap().status,
        Status::Error(AnalysisError::QueryFailed)
    )));
    click("assistant-retry", &mut cx);
    assert_eq!(queries.load(Ordering::SeqCst), 1);
    assert_eq!(models.load(Ordering::SeqCst), 2);
}
