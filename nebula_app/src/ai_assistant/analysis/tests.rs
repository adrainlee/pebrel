use super::*;

fn context(kind: AnalysisKind) -> AnalysisRequest {
    AnalysisRequest {
        kind,
        command: "cargo buld".into(),
        output: "unknown command".into(),
        environment: "remote SSH; OS unknown".into(),
        cwd: "/project".into(),
        exit_code: Some(101),
        language: "Chinese".into(),
        history: Vec::new(),
        query_scope: None,
    }
}

#[test]
fn context_is_bounded_redacted_and_keeps_remote_identity() {
    let mut request = context(AnalysisKind::Logs);
    request.output = format!("{}\nsecret={}\n末尾", "中".repeat(MAX_CONTEXT_BYTES), "a".repeat(64));
    let value: serde_json::Value = serde_json::from_str(&user_context(&request).unwrap()).unwrap();
    assert!(value["output_tail"].as_str().unwrap().len() <= MAX_CONTEXT_BYTES);
    assert!(value["output_tail"].as_str().unwrap().ends_with("末尾"));
    assert!(value["output_tail"].as_str().unwrap().contains("[redacted]"));
    assert!(!value.to_string().contains(&"a".repeat(64)));
    assert_eq!(value["environment"], "remote SSH; OS unknown");
    assert_eq!(value["task"], "analyze_logs");
    assert_eq!(value["current_directory"], "/project");
    for limit in 0..12 {
        assert!(bounded_tail("中文😀hello", limit).len() <= limit);
    }
}

#[test]
fn conversation_keeps_only_recent_bounded_redacted_exchanges() {
    let mut request = context(AnalysisKind::Chat);
    let mut history = Vec::new();
    for index in 0..10 {
        request.command = format!("question {index} {}", "s".repeat(64));
        remember(
            &mut history,
            &request,
            &AnalysisResult {
                analysis: format!("reply {index} {}", "中".repeat(6000)),
                command: None,
                danger: false,
                query: None,
            },
        );
    }
    assert_eq!(history.len(), MAX_TURNS);
    assert!(history[0].question.starts_with("question 4"));
    assert!(history.iter().all(|turn| turn.question.len() <= 4096 && turn.answer.len() <= 4096));
    request.command = "What should I try next?".into();
    request.history = history;
    let value: serde_json::Value = serde_json::from_str(&user_context(&request).unwrap()).unwrap();
    assert_eq!(value["task"], "terminal_conversation");
    assert_eq!(value["conversation"].as_array().unwrap().len(), MAX_TURNS);
    assert!(!value.to_string().contains(&"s".repeat(64)));
    request.command.clear();
    assert_eq!(user_context(&request), Err(AnalysisError::EmptyContext));
}

#[test]
fn blank_or_missing_context_is_explicit() {
    let mut request = context(AnalysisKind::Explain);
    request.command.clear();
    assert_eq!(user_context(&request), Err(AnalysisError::EmptyContext));
    request.kind = AnalysisKind::Logs;
    assert!(user_context(&request).is_ok());
    request.output.clear();
    assert_eq!(user_context(&request), Err(AnalysisError::EmptyContext));
}

#[test]
fn diagnostic_queries_are_validated_independently_of_suggested_commands() {
    let result = parse(r#"{"analysis":"Read the service log first.","command":"reboot","danger":false,"query":{"kind":"journal","unit":"nginx","minutes":30,"lines":200}}"#).unwrap();
    assert!(matches!(result.query, Some(super::super::query::Query::Journal { .. })));
    for query in [
        serde_json::json!({"kind":"shell","command":"whoami"}),
        serde_json::json!({"kind":"file_tail","path":"/var/log/app","shell":"sh"}),
        serde_json::json!({"kind":"journal","unit":"nginx","minutes":0,"lines":200}),
    ] {
        assert_eq!(
            parse(&serde_json::json!({"analysis":"query", "query":query}).to_string()),
            Err(AnalysisError::InvalidResponse)
        );
    }
    let mut request = context(AnalysisKind::Chat);
    request.query_scope = Some("SSH fixture@server".into());
    request.output = "approved query result: timeout".into();
    let mut history = Vec::new();
    remember(&mut history, &request, &result);
    assert!(history[0].question.contains("approved query result: timeout"));
    assert!(user_context(&request).unwrap().contains("SSH fixture@server"));
}

#[test]
fn truncation_does_not_expose_the_suffix_of_a_long_token() {
    let mut request = context(AnalysisKind::Logs);
    request.output = format!("{} {}", "s".repeat(64), ".".repeat(MAX_CONTEXT_BYTES - 21));
    let value: serde_json::Value = serde_json::from_str(&user_context(&request).unwrap()).unwrap();
    let output = value["output_tail"].as_str().unwrap();
    assert!(!output.contains(&"s".repeat(20)), "the original token was long enough to redact");
    assert!(output.contains("[redacted]"));
    assert!(output.len() <= MAX_CONTEXT_BYTES);
    assert_eq!(redacted_tail("prefix short", 5), "short");
    assert_eq!(redacted_tail("中文😀", 4), "😀");
    assert_eq!(redacted_tail("anything", 0), "");
}

#[test]
fn analysis_without_a_correction_is_a_success() {
    let result = parse(r#"{"analysis":"The log contains a timeout; check server reachability.","command":null,"danger":false}"#).unwrap();
    assert!(result.command.is_none());
    assert!(!result.danger);
    assert!(
        parse(
            r#"```json
{"analysis":"拼写错误","command":"cargo build","danger":false}
```"#
        )
        .is_ok()
    );
}

#[test]
fn model_commands_cannot_inject_enter_escape_or_other_controls() {
    for command in [
        "echo hi\recho bad",
        "echo hi\necho bad",
        "echo \x1b[200~bad",
        "echo\0bad",
        "echo\tbad",
        "echo hi\n",
        "\techo hi",
    ] {
        let json =
            serde_json::json!({"analysis":"test", "command":command,"danger":false}).to_string();
        assert_eq!(parse(&json), Err(AnalysisError::InvalidResponse));
    }
    let danger =
        parse(r#"{"analysis":"test","command":"git reset --hard","danger":false}"#).unwrap();
    assert!(danger.danger);
    assert!(parse(r#"{"analysis":"test","command":"echo hi"}"#).unwrap().danger);
}

// Exercise the actual HTTP implementation against a disposable loopback server.
// No provider store, keychain or real model endpoint is touched by these tests.
fn exchange(
    kind: crate::ai_providers::ProviderKind,
    full_url: bool,
    status: &str,
    response: String,
) -> (Result<String, AnalysisError>, String, serde_json::Value) {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let status = status.to_owned();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        let mut headers = String::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse::<usize>().unwrap();
            }
            headers.push_str(&line);
        }
        assert!(length > 0 && length < 64 * 1024);
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes).unwrap();
        let body = serde_json::from_slice(&bytes).unwrap();
        write!(socket, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        (headers, body)
    });
    let target = super::super::RequestTarget {
        kind,
        base_url: format!("http://{address}/custom"),
        model: "test-model".into(),
        full_url,
        key: zeroize::Zeroizing::new("fixture-key".into()),
    };
    let result = send_model_request(&target, SYSTEM, "fixture terminal context", 1600);
    let (headers, body) = server.join().unwrap();
    (result, headers, body)
}

#[test]
fn configured_protocols_keep_route_auth_prompt_and_budget() {
    use crate::ai_providers::ProviderKind;
    let content = r#"{"analysis":"fixture answer","command":null,"danger":false}"#;
    for kind in [
        ProviderKind::OpenAi,
        ProviderKind::Anthropic,
        ProviderKind::Google,
        ProviderKind::AzureOpenAi,
    ] {
        let (reply, path, auth) = match kind {
            ProviderKind::Anthropic => (
                serde_json::json!({"content":[{"text":content}]}),
                "/custom/messages",
                "x-api-key: fixture-key",
            ),
            ProviderKind::Google => (
                serde_json::json!({"candidates":[{"content":{"parts":[{"text":content}]}}]}),
                "/custom/models/test-model:generateContent",
                "x-goog-api-key: fixture-key",
            ),
            ProviderKind::AzureOpenAi => (
                serde_json::json!({"choices":[{"message":{"content":content}}]}),
                "/custom/test-model/chat/completions?api-version=2024-10-21",
                "api-key: fixture-key",
            ),
            _ => (
                serde_json::json!({"choices":[{"message":{"content":content}}]}),
                "/custom/chat/completions",
                "authorization: bearer fixture-key",
            ),
        };
        let (result, headers, body) = exchange(kind, false, "200 OK", reply.to_string());
        assert_eq!(parse(&result.unwrap()).unwrap().analysis, "fixture answer");
        assert!(headers.starts_with(&format!("POST {path} HTTP/1.1")));
        assert!(headers.to_ascii_lowercase().contains(auth));
        match kind {
            ProviderKind::Anthropic => {
                assert_eq!(body["system"], SYSTEM);
                assert_eq!(body["max_tokens"], 1600);
                assert_eq!(body["messages"][0]["content"], "fixture terminal context");
            },
            ProviderKind::Google => {
                assert_eq!(body["systemInstruction"]["parts"][0]["text"], SYSTEM);
                assert_eq!(body["generationConfig"]["maxOutputTokens"], 1600);
                assert_eq!(body["contents"][0]["parts"][0]["text"], "fixture terminal context");
            },
            _ => {
                assert_eq!(body["messages"][0]["content"], SYSTEM);
                assert_eq!(body["max_tokens"], 1600);
                assert_eq!(body["messages"][1]["content"], "fixture terminal context");
            },
        }
    }
}

#[test]
fn full_url_and_http_failures_are_reported_without_a_fake_answer() {
    use crate::ai_providers::ProviderKind;
    let (result, headers, _) =
        exchange(ProviderKind::OpenAi, true, "401 Unauthorized", "{}".into());
    assert_eq!(result, Err(AnalysisError::RequestFailed));
    assert!(headers.starts_with("POST /custom HTTP/1.1"));
    for response in ["not JSON".to_owned(), "{}".to_owned(), "x".repeat(256 * 1024 + 1)] {
        assert_eq!(
            exchange(ProviderKind::OpenAi, true, "200 OK", response).0,
            Err(AnalysisError::InvalidResponse)
        );
    }
}

#[test]
fn redirects_never_forward_provider_credentials() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let stopped = done.clone();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    socket.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
                    let mut reader = BufReader::new(socket.try_clone().unwrap());
                    let mut headers = String::new();
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                            break;
                        }
                        headers.push_str(&line);
                    }
                    write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                    )
                    .unwrap();
                    return Some(headers);
                },
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {},
                Err(error) => panic!("redirect fixture: {error}"),
            }
            if stopped.load(Ordering::Acquire) || std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    });
    let status = format!("302 Found\r\nLocation: http://{address}/unselected-provider");
    let result =
        exchange(crate::ai_providers::ProviderKind::Anthropic, true, &status, "{}".into()).0;
    done.store(true, Ordering::Release);
    let forwarded = server.join().unwrap();
    assert!(
        forwarded.is_none(),
        "a different endpoint received the provider request: {forwarded:?}"
    );
    assert_eq!(result, Err(AnalysisError::RequestFailed));
}

#[test]
fn multipart_provider_answers_exclude_thoughts_and_keep_all_answer_text() {
    use crate::ai_providers::ProviderKind;
    for (kind, reply) in [
        (
            ProviderKind::Anthropic,
            serde_json::json!({"content":[
                {"type":"thinking","thinking":"private reasoning"},
                {"type":"text","text":"{\"analysis\":\""},
                {"type":"text","text":"complete answer\",\"command\":null,\"danger\":false}"}
            ]}),
        ),
        (
            ProviderKind::Google,
            serde_json::json!({"candidates":[{"content":{"parts":[
                {"thought":true,"text":"private reasoning"},
                {"text":"{\"analysis\":\""},
                {"text":"complete answer\",\"command\":null,\"danger\":false}"}
            ]}}]}),
        ),
    ] {
        let text = exchange(kind, false, "200 OK", reply.to_string()).0.unwrap();
        assert_eq!(parse(&text).unwrap().analysis, "complete answer");
        assert!(!text.contains("private reasoning"));
    }
}

#[test]
fn malformed_and_oversized_results_are_explicit() {
    for source in ["plain text", "{}", "}} {{{", r#"{"analysis":" "}"#, r#"{"analysis":false}"#] {
        assert_eq!(parse(source), Err(AnalysisError::InvalidResponse));
    }
    assert_eq!(parse(&"x".repeat(MAX_RESPONSE_BYTES + 1)), Err(AnalysisError::InvalidResponse));
}

#[test]
fn openai_request_uses_task_prompt_and_budget() {
    let body = super::super::openai_body("configured-model", SYSTEM, "selected logs", 1600);
    assert_eq!(body["max_tokens"], 1600);
    assert_eq!(body["messages"][0]["content"], SYSTEM);
    assert_eq!(body["messages"][1]["content"], "selected logs");
}

#[test]
fn short_credential_context_survives_truncation_only_as_redacted_text() {
    let mut request = context(AnalysisKind::Failure);
    request.command = "tool --password demo-pass".into();
    request.output = "Authorization: Bearer demo-short-token".into();
    request.cwd = "postgres://user:demo-db-pass@host/db".into();
    request.history.push(ConversationTurn {
        question: "api_key=demo-short-key".into(),
        answer: "password=demo-again".into(),
    });
    let payload = user_context(&request).unwrap();
    for secret in ["demo-pass", "demo-short-token", "demo-db-pass", "demo-short-key", "demo-again"]
    {
        assert!(!payload.contains(secret), "{secret}: {payload}");
    }
    assert!(!redacted_tail("password=demo-pass", 9).contains("demo-pass"));
    assert!(
        !redacted_tail(&format!("password=\"{}demo-pass\"", " ".repeat(10_000)), 12)
            .contains("demo-pass")
    );
}
