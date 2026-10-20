use super::*;

#[test]
fn only_structured_bounded_reads_are_accepted() {
    for raw in [
        r#"{"kind":"shell","command":"rm -rf /"}"#,
        r#"{"kind":"file_tail","path":"/tmp/log","command":"touch /tmp/marker"}"#,
    ] {
        assert!(serde_json::from_str::<Query>(raw).is_err());
    }
    for query in [
        Query::FileTail { path: "relative.log".into() },
        Query::FileTail { path: "/tmp/log\nwhoami".into() },
        Query::Journal { unit: "nginx;reboot".into(), minutes: 30, lines: 200 },
        Query::DockerLogs { container: "--help".into(), minutes: 30, lines: 200 },
        Query::Journal { unit: "nginx".into(), minutes: 0, lines: 200 },
        Query::Journal { unit: "nginx".into(), minutes: 1441, lines: 200 },
        Query::DockerLogs { container: "web".into(), minutes: 30, lines: 501 },
    ] {
        assert!(!query.validate(), "{query:?}");
    }
    let query = Query::FileTail { path: "/tmp/a'$(touch marker);*.log".into() };
    assert!(query.validate());
    assert_eq!(query.argv().last().unwrap(), "/tmp/a'$(touch marker);*.log");
    assert_eq!(query.command(), "'tail' '-c' '16384' '--' '/tmp/a'\\''$(touch marker);*.log'");
}

fn local(cwd: &std::path::Path) -> Source {
    let options = nebula_terminal::tty::Options {
        working_directory: Some(cwd.to_owned()),
        ..Default::default()
    };
    Source::Local(crate::runtime_exec::PaneExecContext::from_pty_options(&options))
}

#[test]
fn native_file_query_reads_tail_redacts_and_keeps_literal_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("literal'\u{2019}$name;log.txt");
    std::fs::write(
        &path,
        format!("{}\npassword=demo-private\nlast evidence\n", "old line\n".repeat(5000)),
    )
    .unwrap();
    let query = Query::FileTail { path: path.to_string_lossy().into_owned() };
    let source = local(directory.path());
    let output = source
        .read(&query, directory.path().to_str().unwrap(), Arc::new(AtomicBool::new(false)))
        .unwrap();
    assert!(output.contains("last evidence"), "{output}");
    assert!(!output.contains("demo-private"));
    assert!(output.len() <= analysis::MAX_CONTEXT_BYTES);
    assert!(output.contains("exit_code"));
}

#[test]
fn cancelled_query_never_starts_and_missing_file_keeps_failure_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let source = local(directory.path());
    let query = Query::FileTail {
        path: directory.path().join("missing.log").to_string_lossy().into_owned(),
    };
    let cancellation = Cancellation::default();
    let flag = cancellation.0.clone();
    drop(cancellation);
    assert_eq!(
        source.read(&query, directory.path().to_str().unwrap(), flag),
        Err(AnalysisError::QueryFailed)
    );
    let result = source
        .read(&query, directory.path().to_str().unwrap(), Arc::new(AtomicBool::new(false)))
        .unwrap();
    assert!(result.contains("exit_code"));
    assert!(!result.contains("exit_code=0"));
}

#[test]
fn windows_decoder_preserves_path_as_data() {
    use base64::Engine;
    let query = Query::FileTail { path: "C:\\logs\\a';Start-Process calc;'.log".into() };
    let argv = windows_argv(&query).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD.decode(argv.last().unwrap()).unwrap();
    let units = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>();
    let script = String::from_utf16(&units).unwrap();
    assert!(!script.contains("Start-Process"));
    assert!(script.contains(
        &base64::engine::general_purpose::STANDARD.encode("C:\\logs\\a';Start-Process calc;'.log")
    ));
    assert!(script.contains("'Open','Read','ReadWrite'"));
    assert_eq!(argv[1..4], ["-NoProfile", "-NonInteractive", "-EncodedCommand"]);
}
