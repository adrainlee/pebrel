use super::*;

#[test]
fn provider_selection_requires_opt_in_and_never_falls_back_to_another_endpoint() {
    use crate::ai_providers::{AiProvider, ProviderKind, ProviderStore};
    let mut selected = AiProvider::preset(ProviderKind::Ollama, "selected");
    let mut other = AiProvider::preset(ProviderKind::Ollama, "other");
    other.command_completion = true;
    let mut store =
        ProviderStore { active_id: selected.id.clone(), providers: vec![selected.clone(), other] };
    assert!(selected_provider(&store).is_none());
    selected.command_completion = true;
    store.providers[0] = selected;
    assert_eq!(selected_provider(&store).unwrap().id, "selected");
    store.providers[0].enabled = false;
    assert!(selected_provider(&store).is_none());
}

#[test]
fn requests_require_a_completed_word_and_skip_sensitive_or_control_input() {
    for line in [
        "",
        "g",
        "git",
        " git ",
        "git\t",
        "git\nstatus ",
        "echo password=demo ",
        "curl https://user:pass@host ",
        "\"unfinished command ",
        "& ",
    ] {
        assert!(completed_words(line).is_none(), "{line:?}");
    }
    for (line, key) in [
        ("git ", "git "),
        ("git sta", "git "),
        ("Get-ChildItem ", "Get-ChildItem "),
        ("git switch 中", "git switch "),
    ] {
        assert_eq!(completed_words(line), Some(key));
    }
    assert!(completed_words(&format!("git {}", "x".repeat(600))).is_none());
}

#[test]
fn model_can_only_extend_the_literal_prefix_with_bounded_plain_text() {
    assert_eq!(parse("git st", r#"{"command":"git status"}"#).unwrap(), Some("git status".into()));
    assert_eq!(parse("git ", r#"{"command":null}"#).unwrap(), None);
    for command in [
        "cargo build",
        "git status\nwhoami",
        "git status\u{1b}[31m",
        "git push --force",
        "git -c password=demo status",
    ] {
        let response = serde_json::json!({"command":command}).to_string();
        assert_eq!(parse("git ", &response).unwrap(), None, "{command:?}");
    }
    assert!(parse("git ", "```json\n{}\n```").is_err());
    assert!(parse("git ", &"x".repeat(4097)).is_err());
    assert_eq!(suffix("echo 中", "echo 中文"), Some("文"));
}

#[test]
fn shared_admission_stays_busy_until_worker_finishes_and_backs_off_failures() {
    let mut gate = Gate::default();
    let now = Instant::now();
    assert!(gate.admit(now));
    assert!(
        !gate.admit(now + Duration::from_secs(60)),
        "elapsed time never releases an active request"
    );
    gate.finish(now, false);
    assert!(!gate.admit(now + COOLDOWN - Duration::from_millis(1)));
    assert!(gate.admit(now + COOLDOWN));
    gate.finish(now + COOLDOWN, true);
    assert!(!gate.admit(now + COOLDOWN + FAILURE_BACKOFF - Duration::from_millis(1)));
    assert!(gate.admit(now + COOLDOWN + FAILURE_BACKOFF));
}

#[test]
fn request_bounds_context_and_omits_sensitive_history() {
    let request = Request {
        line: "git ".into(),
        cwd: "/work".into(),
        environment: "remote OS unknown".into(),
        history: vec!["git status".into(), "curl --password demo".into(), "x".repeat(600)],
        advice: Some("git status --short".into()),
    };
    let value: serde_json::Value = serde_json::from_str(&payload(&request).unwrap()).unwrap();
    assert_eq!(value["related_history"], serde_json::json!(["git status"]));
    assert_eq!(value["previous_advice"], "git status --short");
    let mut v = serde_json::to_value(crate::ai_providers::AiProvider::preset(
        crate::ai_providers::ProviderKind::Ollama,
        "old",
    ))
    .unwrap();
    v.as_object_mut().unwrap().remove("command_completion");
    let provider: crate::ai_providers::AiProvider = serde_json::from_value(v).unwrap();
    assert!(!provider.command_completion, "old configurations never opt into automatic requests");
}
