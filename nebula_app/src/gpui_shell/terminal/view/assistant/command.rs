//! Explicit slash entry and one-shot submission of the reviewed command.
use super::*;
use std::sync::Weak;
use std::time::Duration;

#[derive(Clone)]
pub(super) struct Scope {
    term: Weak<nebula_terminal::sync::FairMutex<nebula_terminal::Term<session::EventProxy>>>,
    epoch: u64,
    revision: u64,
    cwd: String,
    environment: crate::display::SuggestEnv,
    provider: u64,
    prompt: String,
    started: Option<std::time::Instant>,
}

impl Scope {
    pub(super) fn label(&self) -> String {
        self.prompt.clone()
    }
}

impl TerminalView {
    fn assistant_prompt_available(&self) -> bool {
        // The local ssh process stays alive while the remote shell is idle.
        // This exception is only for a tracked interactive login; a remote
        // command, password prompt or application still needs screen evidence.
        let connection = matches!(
            &self.suggest.suggest_env,
            crate::display::SuggestEnv::Shell {
                scope: crate::nebula_history::HistoryScope::Ssh(_)
            }
        ) && crate::ssh::ssh_destination(&self.suggest.last_committed).is_some();
        !(self.exited.is_some()
            || self.error.is_some()
            || self.marked_text.is_some()
            || self.command_running && !self.command_running_disproved && !connection
            || self.pending_runtime_submit.is_some()
            || self.pending_shell_command.is_some()
            || self
                .running_program
                .as_deref()
                .and_then(crate::ai_agents::AgentKind::parse)
                .is_some()
            || self
                .ssh_stage
                .as_ref()
                .is_some_and(|stage| *stage != crate::ssh_session::SshStage::Ready)
            || self.term_mode().intersects(
                TermMode::ALT_SCREEN | TermMode::VI | TermMode::KITTY_KEYBOARD_PROTOCOL,
            ))
    }

    fn assistant_prompt_line(&self) -> Option<(String, String)> {
        if !self.assistant_prompt_available() {
            return None;
        }
        let term = self.session.as_ref()?.term.lock();
        if let Some(snapshot) = self.completion_editor.snapshot(self.prompt_input_epoch) {
            if snapshot.cursor != snapshot.line.len() {
                return None;
            }
        }
        let line = crate::display::nebula_prompt_line_from_raw_grid(
            &term,
            term.grid().cursor.point,
            &self.suggest.line_buf,
            &self.suggest.suggest_env,
        );
        if let Some(line) = line {
            return Some((line.prompt.trim_end().to_owned(), line.input));
        }
        // SSH servers need not install Pebrel's OSC 133/editor integration.
        // Only an empty mirror and an independently recognized empty prompt
        // permit this fallback; never treat pending deletion as acknowledged.
        if !self.suggest.suggest_env.is_this_machine() && self.suggest.line_buf.is_empty() {
            return crate::display::nebula_idle_prompt_from_raw_grid(
                &term,
                &self.suggest.suggest_env,
            )
            .map(|prompt| (prompt.trim_end().to_owned(), String::new()));
        }
        None
    }

    pub(super) fn assistant_prompt_input(&self) -> Option<String> {
        self.assistant_prompt_line().map(|(_, input)| input)
    }

    pub(super) fn assistant_command_scope(&self, _: &App) -> Option<Scope> {
        let (prompt, input) = self.assistant_prompt_line()?;
        if input.is_empty() && self.suggest.line_buf.is_empty() {
            Some(Scope {
                term: Arc::downgrade(&self.session.as_ref()?.term),
                epoch: self.prompt_input_epoch,
                revision: self.completion_editor.revision(),
                cwd: self.cwd.clone(),
                environment: self.suggest.suggest_env.clone(),
                provider: crate::ai_providers::config_generation(),
                prompt,
                started: self.command_started,
            })
        } else {
            None
        }
    }

    pub(super) fn assistant_scope_matches(&self, scope: &Scope, _: &App) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| scope.term.ptr_eq(&Arc::downgrade(&session.term)))
            && scope.epoch == self.prompt_input_epoch
            && scope.revision == self.completion_editor.revision()
            && scope.cwd == self.cwd
            && scope.environment == self.suggest.suggest_env
            && scope.provider == crate::ai_providers::config_generation()
            && scope.started == self.command_started
    }

    pub(in crate::gpui_shell::terminal::view) fn open_ai_command(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.sync_native_prompt();
        if self.assistant_prompt_input().as_deref() != Some("/ai") {
            return false;
        }
        let key = gpui::Keystroke::parse("backspace").unwrap();
        let Some(backspace) = super::super::super::keymap::encode(&key, &self.term_mode()) else {
            return false;
        };
        // Consume only the exact application command. The shell receives its
        // normal deletion keys, never Enter for /ai. Paths keep ordinary input.
        self.write_input(backspace.repeat(3), cx);
        crate::display::nebula_clear_line(&mut self.suggest);
        self.suggest.expect_completion_echo("/ai".into(), String::new());
        self.open_assistant(AnalysisKind::Command, None, window, cx);
        true
    }

    pub(in crate::gpui_shell::terminal::view) fn invalidate_assistant_command(&mut self) {
        if self.assistant.panel.as_ref().is_some_and(|panel| panel.kind == AnalysisKind::Command) {
            self.assistant.cancel();
            self.assistant.panel.as_mut().unwrap().status = Status::CommandUnavailable;
        }
    }

    pub(super) fn assistant_can_execute(&self, cx: &App) -> bool {
        let Some(panel) = &self.assistant.panel else { return false };
        panel.kind == AnalysisKind::Command
            && matches!(&panel.status, Status::Ready(result) if result.command.as_ref().is_some_and(|command| {
                crate::ai_assistant::valid_suggested_command(command)
                    && self.completion_editor.permits_insert(command)
            }))
            && panel.command_scope.as_ref().is_some_and(|scope| {
                self.assistant_scope_matches(scope, cx)
                    && self
                        .assistant_prompt_line()
                        .is_some_and(|(prompt, input)| prompt == scope.prompt && input.is_empty())
            })
            && self.suggest.line_buf.is_empty()
    }

    pub(super) fn execute_assistant_command(
        &mut self,
        generation: u64,
        command: &str,
        cx: &mut Context<Self>,
    ) {
        if generation != self.assistant.generation || !self.assistant_can_execute(cx) {
            return;
        }
        let panel = self.assistant.panel.as_mut().unwrap();
        if !matches!(&panel.status, Status::Ready(result) if result.command.as_deref() == Some(command))
        {
            return;
        }
        let mut scope = panel.command_scope.clone().unwrap();
        panel.status = Status::Submitting;
        // The preview authorizes this pane only. Use the ordinary paste encoder,
        // then wait for exact shell echo before sending a separate Enter.
        self.paste_now_impl(command, false, cx);
        scope.epoch = self.prompt_input_epoch;
        scope.revision = self.completion_editor.revision();
        let command = command.to_owned();
        self.assistant.task = Some(cx.spawn(async move |this, cx| {
            for _ in 0..150 {
                cx.background_executor().timer(Duration::from_millis(20)).await;
                let waiting = this
                    .update(cx, |view, cx| {
                        if view.assistant.generation != generation {
                            return false;
                        }
                        if !view.assistant_scope_matches(&scope, cx)
                            || !view.assistant_prompt_available()
                        {
                            view.assistant.panel.as_mut().unwrap().status =
                                Status::CommandUnavailable;
                            cx.notify();
                            return false;
                        }
                        let Some((prompt, input)) = view.assistant_prompt_line() else {
                            // An SSH round trip may deliver only part of the
                            // paste. Wait for full echo instead of aborting it.
                            return true;
                        };
                        if prompt != scope.prompt {
                            view.assistant.panel.as_mut().unwrap().status =
                                Status::CommandUnavailable;
                            cx.notify();
                            return false;
                        }
                        if input != command {
                            return true;
                        }
                        let key = gpui::Keystroke::parse("enter").unwrap();
                        let mode = view.term_mode();
                        let Some(bytes) = super::super::super::keymap::encode(&key, &mode) else {
                            return true;
                        };
                        view.track_encoded_key(&key, &mode, cx);
                        view.write_input(bytes, cx);
                        let panel = view.assistant.panel.as_mut().unwrap();
                        panel.command_scope = None;
                        panel.status = Status::Submitted;
                        cx.notify();
                        false
                    })
                    .unwrap_or(false);
                if !waiting {
                    return;
                }
            }
            let _ = this.update(cx, |view, cx| {
                if view.assistant.generation == generation {
                    view.assistant.panel.as_mut().unwrap().status = Status::SubmitFailed;
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }
}
