//! Approval binds one structured query to the pane snapshot that produced it.
use super::*;
use crate::ai_assistant::query::{Cancellation, Query, Source};
use crate::display::SuggestEnv;
use std::sync::{Mutex, Weak};

#[derive(Clone)]
pub(super) struct Target {
    owner: Weak<Mutex<Option<crate::ssh_session::TranscriptReader>>>,
    environment: SuggestEnv,
    cwd: String,
    source: Source,
    pub(super) label: String,
}

impl TerminalView {
    pub(super) fn assistant_query_target(&self) -> Option<Target> {
        if self.exited.is_some() || self.error.is_some() {
            return None;
        }
        let session = self.session.as_ref()?;
        let (source, label) = match &self.suggest.suggest_env {
            SuggestEnv::Ssh { destination }
                if self.ssh_destination.as_ref() == Some(destination) =>
            {
                let reader = session.remote_reader.lock().ok()?.clone()?;
                if !reader.is_live() {
                    return None;
                }
                (Source::Ssh(reader), format!("SSH {destination}"))
            },
            SuggestEnv::Local if self.ssh_destination.is_none() => {
                let context = self.exec_context.clone()?;
                if context.wsl_distribution().is_some() {
                    return None;
                }
                (Source::Local(context), format!("{} · {}", std::env::consts::OS, self.cwd))
            },
            SuggestEnv::Wsl { distro } if self.ssh_destination.is_none() => {
                let context = self.exec_context.as_ref()?.for_wsl_distribution(distro)?;
                (Source::Local(context), format!("WSL {distro} · {}", self.cwd))
            },
            // A manually typed ssh/su/container shell has no verified exec capability.
            _ => return None,
        };
        Some(Target {
            owner: Arc::downgrade(&session.remote_reader),
            environment: self.suggest.suggest_env.clone(),
            cwd: self.cwd.clone(),
            source,
            label,
        })
    }

    fn assistant_query_target_matches(&self, target: &Target) -> bool {
        let Some(current) = self.assistant_query_target() else { return false };
        target.owner.ptr_eq(&current.owner)
            && target.environment == current.environment
            && target.cwd == current.cwd
            && match (&target.source, &current.source) {
                (Source::Ssh(old), Source::Ssh(new)) => old.same_scope(new),
                (Source::Local(_), Source::Local(_)) => true,
                _ => false,
            }
    }

    pub(super) fn assistant_can_query(&self, query: &Query) -> bool {
        self.assistant.task.is_none()
            && self.assistant.query_target.as_ref().is_some_and(|target| {
                target.source.supports(query) && self.assistant_query_target_matches(target)
            })
    }

    pub(super) fn authorize_assistant_query(
        &mut self,
        expected: u64,
        query: Query,
        cx: &mut Context<Self>,
    ) {
        if expected != self.assistant.generation || !self.assistant_can_query(&query)
            || !self.assistant.panel.as_ref().is_some_and(|panel|
                matches!(&panel.status, Status::Ready(result) if result.query.as_ref() == Some(&query))) {
            return;
        }
        let target = self.assistant.query_target.clone().unwrap();
        let Some(mut request) = self.assistant.last_request.clone() else { return };
        self.assistant.cancel();
        let generation = self.assistant.generation;
        let cancellation = Cancellation::default();
        let cancelled = cancellation.0.clone();
        self.assistant.query_cancellation = Some(cancellation);
        self.assistant.panel.as_mut().unwrap().status = Status::Querying;
        request.kind = AnalysisKind::Chat;
        // Keep the user's question and the previous exchanges while attaching actual evidence.
        if request.command.trim().is_empty() {
            request.command = "Analyze the approved diagnostic query result.".into();
        }
        request.history = self.assistant.history.clone();
        let description = serde_json::to_string(&query).expect("diagnostic query is serializable");
        #[cfg(test)]
        let worker = self.assistant.query_worker.clone();
        self.assistant.task = Some(cx.spawn(async move |view, cx| {
            let source = target.source.clone();
            let cwd = target.cwd.clone();
            let result = cx
                .background_spawn(async move {
                    #[cfg(test)]
                    if let Some(worker) = worker {
                        return worker(&query, cancelled);
                    }
                    source.read(&query, &cwd, cancelled)
                })
                .await;
            let _ = view.update(cx, |view, cx| {
                if view.assistant.generation != generation {
                    return;
                }
                view.assistant.task = None;
                view.assistant.query_cancellation = None;
                if !view.assistant_query_target_matches(&target) {
                    view.assistant.panel.as_mut().unwrap().status =
                        Status::Error(AnalysisError::QueryFailed);
                    cx.notify();
                    return;
                }
                match result {
                    Ok(output) => {
                        request.output = format!(
                            "Approved query on {}: {description}\n{}",
                            target.label,
                            analysis::redacted_tail(
                                &output,
                                analysis::MAX_CONTEXT_BYTES
                                    .saturating_sub(target.label.len() + description.len() + 32)
                            )
                        );
                        view.start_assistant(request, false, cx);
                    },
                    Err(error) => {
                        view.assistant.panel.as_mut().unwrap().status = Status::Error(error);
                        cx.notify();
                    },
                }
            });
        }));
        cx.notify();
    }
}
