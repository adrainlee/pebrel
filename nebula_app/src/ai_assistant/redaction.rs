//! Best-effort protection of terminal context before provider requests.
//! Field names, URL userinfo and known credential shapes supplement entropy.
use regex::Regex;
use std::sync::LazyLock;

pub fn redact_secrets(text: &str) -> String {
    static PRIVATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"(?s)-----BEGIN (?:[A-Z]+ )?PRIVATE KEY-----.*?(?:-----END (?:[A-Z]+ )?PRIVATE KEY-----|$)"
    ).unwrap()
    });
    static AUTH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)(\b(?:bearer|basic)\s+)[^\s"',;]+"#).unwrap());
    static URL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)([a-z][a-z0-9+.-]*://)[^/\s@]+@").unwrap());
    static FIELD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r#"(?ix)
        (\b(?:[a-z0-9]+[_-])*(?:password|passwd|pwd|passphrase|token|secret|credential|api[_-]?key|access[_-]?key(?:[_-]?id)?|secret[_-]?key|private[_-]?key)
        ["']?\s*(?:=|:|\s)\s*)
        (?:"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|[^\s,;"'}]+)
        "#
    ).unwrap()
    });
    static KNOWN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"\b(?:(?:AKIA|ASIA)[A-Z0-9]{16}|(?:gh[pousr]_|github_pat_|glpat-|sk-)[A-Za-z0-9_-]{8,})\b"
    ).unwrap()
    });
    let text = PRIVATE_KEY.replace_all(text, "[redacted]");
    let text = AUTH.replace_all(&text, "${1}[redacted]");
    let text = URL.replace_all(&text, "${1}[redacted]@");
    let text = FIELD.replace_all(&text, "${1}[redacted]");
    let text = KNOWN.replace_all(&text, "[redacted]");
    // Retain the old fallback for unknown long keys, without a second run buffer.
    let mut output = String::with_capacity(text.len());
    let mut start = 0;
    for (offset, ch) in text.char_indices() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '+' | '/' | '=' | '-' | '_') {
            continue;
        }
        append_run(&mut output, &text[start..offset]);
        output.push(ch);
        start = offset + ch.len_utf8();
    }
    append_run(&mut output, &text[start..]);
    output
}

fn append_run(output: &mut String, run: &str) {
    output.push_str(if run.len() >= 40 { "[redacted]" } else { run });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_credentials_are_removed_without_erasing_nonsecret_context() {
        for (input, secret) in [
            ("password=demo-pass path=C:/work", "demo-pass"),
            ("--password demo-pass --verbose", "demo-pass"),
            (r#"{"api_key":"short-demo", "model":"example"}"#, "short-demo"),
            ("Authorization: Bearer short-demo-token", "short-demo-token"),
            ("Authorization: Basic dXNlcjpwYXNz", "dXNlcjpwYXNz"),
            ("AWS_ACCESS_KEY_ID=AKIA1234567890ABCDEF", "AKIA1234567890ABCDEF"),
            ("AWS_SECRET_ACCESS_KEY=short-demo", "short-demo"),
            ("postgres://demo:db-pass@db.example/test", "db-pass"),
            ("url?access_token=short-demo&next=1", "short-demo"),
            ("-----BEGIN PRIVATE KEY-----\nshort\n-----END PRIVATE KEY-----", "short"),
        ] {
            let output = redact_secrets(input);
            assert!(!output.contains(secret), "{input}: {output}");
            assert!(output.contains("[redacted]"));
        }
        assert_eq!(
            redact_secrets("cargo build --release C:/work"),
            "cargo build --release C:/work"
        );
        assert!(redact_secrets("password=demo-pass path=C:/work").contains("path=C:/work"));
        assert_eq!(redact_secrets(&"a".repeat(64)), "[redacted]");
    }
}
