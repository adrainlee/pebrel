//! Structured, bounded diagnostic reads. Model command strings are never executed.
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use super::analysis::{self, AnalysisError};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Query {
    FileTail { path: String },
    ListDirectory { path: String },
    Journal { unit: String, minutes: u16, lines: u16 },
    DockerLogs { container: String, minutes: u16, lines: u16 },
}

impl Query {
    pub(crate) fn validate(&self) -> bool {
        let name = |s: &str| {
            !s.is_empty()
                && s.len() <= 256
                && !s.starts_with('-')
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._@-".contains(&b))
        };
        match self {
            Self::FileTail { path } | Self::ListDirectory { path } => {
                !path.is_empty()
                    && path.len() <= 2048
                    && !path.chars().any(char::is_control)
                    && (path.starts_with('/')
                        || (path.as_bytes().get(1) == Some(&b':')
                            && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                            && matches!(path.as_bytes().get(2), Some(b'/' | b'\\'))))
            },
            Self::Journal { unit, minutes, lines } => name(unit) && bounds(*minutes, *lines),
            Self::DockerLogs { container, minutes, lines } => {
                name(container) && bounds(*minutes, *lines)
            },
        }
    }

    pub(crate) fn argv(&self) -> Vec<String> {
        match self {
            Self::FileTail { path } => {
                // Read preceding context for credential labels just before the sent excerpt.
                vec!["tail".into(), "-c".into(), "16384".into(), "--".into(), path.clone()]
            },
            Self::ListDirectory { path } => {
                vec!["ls".into(), "-1Af".into(), "--".into(), path.clone()]
            },
            Self::Journal { unit, minutes, lines } => vec![
                "journalctl".into(),
                "--no-pager".into(),
                "--output=short-iso".into(),
                format!("--unit={unit}"),
                format!("--since=-{minutes}min"),
                format!("--lines={lines}"),
            ],
            Self::DockerLogs { container, minutes, lines } => vec![
                "docker".into(),
                "logs".into(),
                "--timestamps".into(),
                format!("--since={minutes}m"),
                format!("--tail={lines}"),
                container.clone(),
            ],
        }
    }

    pub(crate) fn command(&self) -> String {
        self.argv().iter().map(|word| quote(word)).collect::<Vec<_>>().join(" ")
    }
}

fn bounds(minutes: u16, lines: u16) -> bool {
    (1..=1440).contains(&minutes) && (1..=500).contains(&lines)
}

fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// Dropping the UI owner cancels I/O even when a background worker still exists.
#[derive(Default)]
pub(crate) struct Cancellation(pub Arc<AtomicBool>);
impl Drop for Cancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[derive(Clone)]
pub(crate) enum Source {
    Ssh(crate::ssh_session::TranscriptReader),
    Local(crate::runtime_exec::PaneExecContext),
}

impl Source {
    pub(crate) fn windows(&self) -> bool {
        cfg!(windows)
            && matches!(self, Self::Local(context) if context.wsl_distribution().is_none())
    }

    pub(crate) fn supports(&self, query: &Query) -> bool {
        query.validate()
            && match query {
                Query::FileTail { path } | Query::ListDirectory { path } => {
                    if self.windows() {
                        path.as_bytes().get(1) == Some(&b':')
                    } else {
                        path.starts_with('/')
                    }
                },
                _ => !self.windows(),
            }
    }

    pub(crate) fn read(
        &self,
        query: &Query,
        cwd: &str,
        cancelled: Arc<AtomicBool>,
    ) -> Result<String, AnalysisError> {
        if !self.supports(query) || cancelled.load(Ordering::Acquire) {
            return Err(AnalysisError::QueryFailed);
        }
        let _slot = analysis::Slot::acquire()?;
        let result = match self {
            Self::Ssh(reader) => {
                let runtime =
                    crate::ssh_session::runtime().map_err(|_| AnalysisError::QueryFailed)?;
                runtime
                    .block_on(reader.capture_command(&query.command(), cancelled))
                    .map_err(|_| AnalysisError::QueryFailed)?
            },
            Self::Local(context) => {
                let argv = if self.windows() { windows_argv(query)? } else { query.argv() };
                let value = crate::runtime_exec::execute_cancellable(
                    context.clone(),
                    cwd.into(),
                    argv,
                    10_000,
                    64 * 1024,
                    Some(cancelled),
                )
                .map_err(|_| AnalysisError::QueryFailed)?;
                if value["timed_out"].as_bool() == Some(true) {
                    return Err(AnalysisError::QueryFailed);
                }
                format!(
                    "stdout:\n{}\nstderr:\n{}\n[query exit_code={} stdout_truncated={} stderr_truncated={}]",
                    value["stdout"].as_str().unwrap_or_default(),
                    value["stderr"].as_str().unwrap_or_default(),
                    value["exit_code"],
                    value["capture"]["stdout"]["truncated"],
                    value["capture"]["stderr"]["truncated"]
                )
            },
        };
        // Redact before keeping a bounded excerpt; raw logs are never persisted.
        const NOTICE: &str = "\n[query excerpt limited to the last 12 KiB]";
        if result.len() > analysis::MAX_CONTEXT_BYTES {
            Ok(format!(
                "{}{NOTICE}",
                analysis::redacted_tail(&result, analysis::MAX_CONTEXT_BYTES - NOTICE.len())
            ))
        } else {
            Ok(analysis::redacted_tail(&result, analysis::MAX_CONTEXT_BYTES))
        }
    }
}

fn windows_argv(query: &Query) -> Result<Vec<String>, AnalysisError> {
    use base64::Engine;
    let (path, body) = match query {
        Query::FileTail { path } => (
            path,
            r#"$f=[IO.File]::Open($path,'Open','Read','ReadWrite'); try { if(-not $f.CanSeek) { throw 'not a regular file' }; $null=$f.Seek([Math]::Max(0,$f.Length-16384),'Begin'); $b=New-Object byte[] 16384; $n=$f.Read($b,0,$b.Length); [Console]::Write([Text.Encoding]::UTF8.GetString($b,0,$n)) } finally { $f.Dispose() }"#,
        ),
        Query::ListDirectory { path } => (
            path,
            r#"Get-ChildItem -Force -LiteralPath $path | Select-Object -First 200 -ExpandProperty Name; Write-Output '[directory listing limited to 200 entries]'"#,
        ),
        _ => return Err(AnalysisError::QueryFailed),
    };
    // Model paths never enter PowerShell source text, including smart quote characters.
    let path = base64::engine::general_purpose::STANDARD.encode(path.as_bytes());
    let script = format!(
        "$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.Encoding]::UTF8; $path=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{path}')); {body}"
    );
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    // Resolve the OS executable, never a model-provided shell or current-directory program.
    let powershell = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:\\Windows".into())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    Ok(vec![
        powershell.to_string_lossy().into_owned(),
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-EncodedCommand".into(),
        base64::engine::general_purpose::STANDARD.encode(bytes),
    ])
}

#[cfg(test)]
mod tests;
