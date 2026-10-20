//! Pane-owned debounce, cache and admission of model completions.
use super::*;
use crate::ai_assistant::prediction::{self as model, Request};
use crate::display::{CompletionStyle, NebulaCompletionItem, NebulaCompletionKind};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

#[derive(Clone, PartialEq)]
struct Identity {
    scope: String,
    line: String,
    epoch: u64,
    revision: u64,
}

struct Pending {
    _task: gpui::Task<()>,
    cancelled: Arc<AtomicBool>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
pub(super) struct Prediction {
    scope: String,
    identity: Option<Identity>,
    pending: Option<Pending>,
    attempted: VecDeque<(String, Instant)>,
    cached: Option<(String, Instant)>,
    advice: Option<String>,
    displayed: Option<String>,
    dismissed: Option<String>,
    #[cfg(test)]
    worker: Option<Arc<dyn Fn(&Request) -> Option<String> + Send + Sync>>,
}

impl Prediction {
    pub(super) fn invalidate(&mut self) {
        self.pending = None;
        self.identity = None;
        self.displayed = None;
    }
    pub(super) fn reset(&mut self) {
        self.invalidate();
        self.scope.clear();
        self.attempted.clear();
        self.cached = None;
        self.advice = None;
        self.dismissed = None;
    }
}

impl TerminalView {
    fn prediction_scope(&self) -> String {
        let owner = self.session.as_ref().map(|s| Arc::as_ptr(&s.term));
        format!(
            "{owner:?}|{:?}|{}|{}",
            self.suggest.suggest_env,
            self.suggest.cwd,
            crate::ai_providers::config_generation()
        )
    }

    fn prediction_allowed(&self) -> bool {
        self.ghost_enabled
            && self.exited.is_none()
            && self.error.is_none()
            && self.marked_text.is_none()
            && !matches!(self.ssh_stage, Some(crate::ssh_session::SshStage::Failed(_)))
            && self.session.as_ref().is_some_and(|s| {
                let term = s.term.lock();
                term.nebula_prompt_active()
                    && !term.mode().intersects(TermMode::ALT_SCREEN | TermMode::VI)
            })
            && self
                .completion_editor
                .snapshot(self.prompt_input_epoch)
                .is_none_or(|s| s.cursor == s.line.len())
    }

    fn prediction_matches(&self, id: &Identity) -> bool {
        self.prediction_allowed()
            && self.prediction_scope() == id.scope
            && self.suggest.screen_line == id.line
            && self.prompt_input_epoch == id.epoch
            && self.completion_editor.revision() == id.revision
            && self.prediction.dismissed.as_deref() != Some(id.line.as_str())
            && self.suggest.completion_suppressed_line.as_deref() != Some(id.line.as_str())
    }

    pub(super) fn remember_prediction_advice(&mut self, command: &str) {
        let scope = self.prediction_scope();
        if self.prediction.scope != scope {
            self.prediction.reset();
            self.prediction.scope = scope;
        }
        if command.len() <= 1024 && crate::ai_assistant::valid_suggested_command(command) {
            self.prediction.advice = Some(command.to_owned());
        }
    }

    pub(super) fn refresh_prediction(&mut self, line: &str, cursor: usize, cx: &mut Context<Self>) {
        let scope = self.prediction_scope();
        if scope != self.prediction.scope {
            self.clear_prediction_display();
            self.prediction.reset();
            self.prediction.scope = scope.clone();
        }
        if !self.prediction_allowed()
            || cursor != line.len()
            || self.suggest.completion_suppressed_line.as_deref() == Some(line)
            || self.prediction.dismissed.as_deref() == Some(line)
        {
            self.clear_prediction_display();
            self.prediction.invalidate();
            return;
        }
        let id = Identity {
            scope,
            line: line.to_owned(),
            epoch: self.prompt_input_epoch,
            revision: self.completion_editor.revision(),
        };
        if self.prediction.identity.as_ref() != Some(&id) {
            self.clear_prediction_display();
            self.prediction.invalidate();
            self.prediction.identity = Some(id.clone());
        }
        if self.apply_prediction() || self.prediction.pending.is_some() {
            return;
        }
        let Some(words) = model::completed_words(line).map(str::to_owned) else { return };
        if self
            .prediction
            .attempted
            .iter()
            .any(|(key, time)| key == &words && time.elapsed() < model::CACHE_TTL)
        {
            return;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let provider_generation = crate::ai_providers::config_generation();
        #[cfg(test)]
        let worker = self.prediction.worker.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(model::DEBOUNCE).await;
            let prepared = this
                .update(cx, |view, _| {
                    if !view.prediction_matches(&id) {
                        return None;
                    }
                    view.prediction.attempted.push_back((words, Instant::now()));
                    while view.prediction.attempted.len() > 16 {
                        view.prediction.attempted.pop_front();
                    }
                    let env = view.suggest.suggest_env.clone();
                    let shell = if env.is_this_machine() {
                        view.exec_context
                            .as_ref()
                            .and_then(|e| e.shell_program())
                            .unwrap_or("unknown")
                    } else {
                        "remote shell; do not infer its OS or dialect from the client"
                    };
                    let request = Request {
                        line: id.line.clone(),
                        cwd: view.suggest.cwd.clone(),
                        environment: format!("scope: {env:?}; shell: {shell}"),
                        history: Vec::new(),
                        advice: view.prediction.advice.clone(),
                    };
                    Some((request, env))
                })
                .ok()
                .flatten();
            let Some((mut request, env)) = prepared else { return };
            let result = cx
                .background_spawn(async move {
                    if worker_cancelled.load(Ordering::Relaxed) {
                        return None;
                    }
                    let word = request.line.split_whitespace().next().unwrap_or("");
                    request.history = crate::completion::prediction_history(&env, word);
                    #[cfg(test)]
                    if let Some(worker) = worker {
                        return worker(&request);
                    }
                    model::predict(&request, &worker_cancelled, provider_generation).ok().flatten()
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.finish_prediction(&id, result, cx);
            });
        });
        self.prediction.pending = Some(Pending { _task: task, cancelled });
    }

    fn finish_prediction(&mut self, id: &Identity, result: Option<String>, cx: &mut Context<Self>) {
        if !self.prediction_matches(id) {
            return;
        }
        self.prediction.pending = None;
        if let Some(command) = result.filter(|s| model::suffix(&id.line, s).is_some()) {
            self.prediction.cached = Some((command, Instant::now()));
            self.apply_prediction();
            cx.notify();
        }
    }

    pub(super) fn apply_prediction(&mut self) -> bool {
        let Some(id) = self.prediction.identity.as_ref() else { return false };
        if !self.prediction_matches(id) {
            return false;
        }
        let Some((command, time)) = &self.prediction.cached else { return false };
        if time.elapsed() >= model::CACHE_TTL {
            return false;
        }
        // The response was validated once on admission. Painting only slices
        // the already validated text; no regex or credential lookup per frame.
        let Some(suffix) = command.strip_prefix(&id.line).filter(|s| !s.is_empty()) else {
            return false;
        };
        if !self.completion_editor.permits_insert(suffix) {
            return false;
        }
        let item = NebulaCompletionItem {
            label: format!("AI: {command}"),
            insert: suffix.to_owned(),
            replace_chars: 0,
            replace_after_chars: 0,
            kind: NebulaCompletionKind::Command,
        };
        if self.completion_style == CompletionStyle::Popup
            || self.suggest.completion_popup_requested
        {
            if !self.suggest.completion_items.iter().any(|i| i.insert == item.insert) {
                self.suggest.completion_items.push(item);
            }
        } else {
            self.suggest.suggestion = suffix.to_owned();
            self.suggest.suggestion_edit = Some(item);
            self.prediction.displayed = Some(suffix.to_owned());
        }
        true
    }

    fn clear_prediction_display(&mut self) {
        if self.prediction.displayed.take().as_ref() == Some(&self.suggest.suggestion) {
            self.suggest.suggestion.clear();
            self.suggest.suggestion_edit = None;
        }
        self.suggest.completion_items.retain(|item| !item.label.starts_with("AI: "));
    }

    pub(super) fn prediction_input(&mut self) {
        self.clear_prediction_display();
        self.prediction.invalidate();
    }

    pub(super) fn prediction_item_valid(&self, item: &NebulaCompletionItem) -> bool {
        if !item.label.starts_with("AI: ") {
            return true;
        }
        self.prediction.identity.as_ref().is_some_and(|id| {
            self.prediction_matches(id)
                && self.prediction.cached.as_ref().is_some_and(|(command, time)| {
                    time.elapsed() < model::CACHE_TTL
                        && command.strip_prefix(&id.line) == Some(item.insert.as_str())
                })
        })
    }

    pub(super) fn handle_prediction_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if key == "escape"
            && (self.prediction.pending.is_some() || self.prediction.displayed.is_some())
        {
            let popup =
                self.suggest.completion_popup_requested || suggest::popup_active(&self.suggest);
            self.prediction.dismissed = Some(self.suggest.screen_line.clone());
            self.prediction_input();
            // Let the existing popup handler close its list in the same keypress.
            return !popup;
        }
        if !matches!(key, "tab" | "right") || self.prediction.displayed.is_none() {
            return false;
        }
        if !self.prediction.identity.as_ref().is_some_and(|id| self.prediction_matches(id)) {
            self.prediction_input();
            return false;
        }
        let Some(item) = self.suggest.suggestion_edit.clone() else { return false };
        if Some(&item.insert) != self.prediction.displayed.as_ref() {
            return false;
        }
        let accepted = self.accept_completion_item(item, cx);
        self.prediction_input();
        accepted
    }
}

#[cfg(all(test, feature = "gpui-test-support"))]
mod tests;
