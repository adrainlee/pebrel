//! Pane-scoped read capability. It never resolves an address or reauthenticates.
use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::sync::watch;

use super::{ClientSession, SessionError, SharedSession, exec};

#[derive(Clone)]
pub struct TranscriptReader {
    session: Weak<ClientSession>,
    closed: watch::Receiver<bool>,
}

pub(super) struct TranscriptScope(watch::Sender<bool>);

impl TranscriptScope {
    pub(super) fn new(session: &SharedSession) -> (Self, TranscriptReader) {
        let (sender, closed) = watch::channel(false);
        (Self(sender), TranscriptReader { session: Arc::downgrade(session), closed })
    }
}

impl Drop for TranscriptScope {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

impl TranscriptReader {
    pub(crate) fn is_live(&self) -> bool {
        !*self.closed.borrow()
            && self.closed.has_changed().is_ok()
            && self.session.upgrade().is_some_and(|session| !session.is_closed())
    }

    pub(crate) fn same_scope(&self, other: &Self) -> bool {
        self.closed.same_channel(&other.closed) && Weak::ptr_eq(&self.session, &other.session)
    }

    /// Bounded, independently owned exec channel on this exact authenticated transport.
    /// Callers compile structured, approved reads; this method does not grant permission.
    pub(crate) async fn capture_command(
        &self,
        command: &str,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<String, SessionError> {
        use russh::ChannelMsg;
        use std::sync::atomic::Ordering;
        if !self.is_live() || cancelled.load(Ordering::Acquire) {
            return Err("query source closed or cancelled".into());
        }
        let session = self.session.upgrade().ok_or("query connection closed")?;
        let mut closed = self.closed.clone();
        let work = async {
            let channel = session.channel_open_session().await?;
            let mut channel = super::lifecycle::own_channel(channel);
            channel.exec(true, command).await?;
            let mut output = Vec::new();
            let mut status = None;
            let mut eof = false;
            let mut truncated = false;
            loop {
                match channel.wait().await {
                    Some(ChannelMsg::Success) => {
                        channel.eof().await?;
                    },
                    Some(ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. }) => {
                        let keep = (64usize * 1024).saturating_sub(output.len()).min(data.len());
                        output.extend_from_slice(&data[..keep]);
                        if keep < data.len() {
                            truncated = true;
                            break;
                        }
                    },
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        status = Some(exit_status);
                        if eof {
                            break;
                        }
                    },
                    Some(ChannelMsg::Eof) => {
                        eof = true;
                        if status.is_some() {
                            break;
                        }
                    },
                    Some(ChannelMsg::Failure) => return Err("query exec rejected".into()),
                    Some(ChannelMsg::Close) | None => break,
                    _ => {},
                }
            }
            channel.finish().await?;
            if status.is_none() && !truncated {
                return Err("query exit status missing".into());
            }
            Ok(format!(
                "{}\n[query exit_code={status:?} truncated={truncated}]",
                String::from_utf8_lossy(&output)
            ))
        };
        let cancellation = async {
            loop {
                if cancelled.load(Ordering::Acquire) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        tokio::select! {
            biased;
            _ = closed.changed() => Err("query source closed".into()),
            _ = cancellation => Err("query cancelled".into()),
            result = tokio::time::timeout(Duration::from_secs(10), work) => result.map_err(|_| "query timed out")?,
        }
    }

    pub(crate) async fn capture(&self, script: &[u8]) -> Result<String, SessionError> {
        if *self.closed.borrow() {
            return Err("SSH transcript owner closed".into());
        }
        let session = self
            .session
            .upgrade()
            .filter(|session| !session.is_closed())
            .ok_or("SSH transcript connection closed")?;
        let mut closed = self.closed.clone();
        let budget = Duration::from_secs(5);
        // 只借用原 pane 的认证连接；换网/重连/关页后不去连接池寻找替代主机。
        tokio::select! {
            biased;
            _ = closed.changed() => Err("SSH transcript owner closed".into()),
            result = tokio::time::timeout(budget, async {
                let channel = session.channel_open_session().await?;
                exec::capture(channel, "python3 -", script, budget, "native transcript").await
            }) => result.map_err(|_| "SSH transcript read timed out")?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ended_or_missing_owner_cannot_open_a_replacement_connection() {
        let (sender, closed) = watch::channel(false);
        let reader = TranscriptReader { session: Weak::new(), closed };
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        assert!(runtime.block_on(reader.capture(b"unreachable")).is_err());
        sender.send_replace(true);
        assert!(runtime.block_on(reader.capture(b"unreachable")).is_err());
    }
}
