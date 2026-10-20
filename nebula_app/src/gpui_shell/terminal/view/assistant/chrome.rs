use super::*;
use crate::gpui_shell::prelude::*;
use crate::i18n::Message;
use gpui::prelude::FluentBuilder;

impl TerminalView {
    pub(in crate::gpui_shell::terminal) fn assistant_chrome(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let panel = self.assistant.panel.as_ref()?;
        let language = crate::gpui_shell::config::ui_language(cx);
        let t = |m| language.text(m);
        let pending = matches!(panel.status, Status::Loading | Status::Querying);
        let mut conversation = v_flex().w_full().gap_3();
        if self.assistant.history.is_empty() && matches!(panel.status, Status::Idle) {
            conversation = conversation.child(
                v_flex()
                    .gap_2()
                    .py_3()
                    .child(div().font_semibold().child(t(Message::AssistantWelcome)))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t(Message::AssistantWelcomeHint)),
                    ),
            );
        }
        for (index, turn) in self.assistant.history.iter().enumerate() {
            conversation = conversation.child(
                v_flex()
                    .id(("assistant-turn", index))
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t(Message::AssistantYou)),
                    )
                    .child(
                        div()
                            .p_2()
                            .rounded_md()
                            .bg(cx.theme().secondary)
                            .text_sm()
                            .child(turn.question.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t(Message::AssistantTitle)),
                    )
                    .child(div().text_sm().child(turn.answer.clone())),
            );
        }
        match &panel.status {
            Status::Idle => {},
            Status::Loading => {
                conversation =
                    conversation.child(div().text_sm().child(t(Message::AssistantAnalyzing)));
            },
            Status::Querying => {
                conversation =
                    conversation.child(div().text_sm().child(t(Message::AssistantQueryLoading)));
            },
            Status::Error(error) => {
                let message = match error {
                    AnalysisError::NoProvider => Message::AssistantNoProvider,
                    AnalysisError::EmptyContext => Message::AssistantEmptyContext,
                    AnalysisError::Busy => Message::AssistantBusy,
                    AnalysisError::RequestFailed => Message::AssistantRequestFailed,
                    AnalysisError::InvalidResponse => Message::AssistantInvalidResponse,
                    AnalysisError::QueryFailed => Message::AssistantQueryFailed,
                };
                conversation = conversation.child(div().text_sm().child(t(message)));
                if *error != AnalysisError::EmptyContext
                    && self
                        .assistant
                        .last_request
                        .as_ref()
                        .is_some_and(|request| request.kind == panel.kind)
                {
                    conversation = conversation.child(
                        Button::new("assistant-retry")
                            .debug_selector(|| "assistant-retry".into())
                            .label(t(Message::CommonRetry))
                            .small()
                            .ghost()
                            .on_click(cx.listener(|view, _, _, cx| {
                                if let Some(request) = view.assistant.last_request.clone() {
                                    view.start_assistant(request, false, cx);
                                }
                            })),
                    );
                }
            },
            Status::Inserted => {
                conversation =
                    conversation.child(div().text_sm().child(t(Message::AssistantInserted)));
            },
            Status::Ready(result) => {
                // Cached automatic advice has no manual conversation entry yet.
                if self
                    .assistant
                    .history
                    .last()
                    .is_none_or(|turn| !turn.answer.starts_with(&result.analysis))
                {
                    conversation =
                        conversation.child(div().text_sm().child(result.analysis.clone()));
                }
                let copy = result.analysis.clone();
                if let Some(query) = &result.query {
                    use crate::ai_assistant::query::Query;
                    let description = match query {
                        Query::FileTail { path } => {
                            language.format(Message::AssistantQueryFile, &[("path", path)])
                        },
                        Query::ListDirectory { path } => {
                            language.format(Message::AssistantQueryDirectory, &[("path", path)])
                        },
                        Query::Journal { unit, minutes, lines } => language.format(
                            Message::AssistantQueryJournal,
                            &[
                                ("name", unit),
                                ("minutes", &minutes.to_string()),
                                ("lines", &lines.to_string()),
                            ],
                        ),
                        Query::DockerLogs { container, minutes, lines } => language.format(
                            Message::AssistantQueryDocker,
                            &[
                                ("name", container),
                                ("minutes", &minutes.to_string()),
                                ("lines", &lines.to_string()),
                            ],
                        ),
                    };
                    let target = self
                        .assistant
                        .query_target
                        .as_ref()
                        .map(|t| t.label.as_str())
                        .unwrap_or_default();
                    let allowed = self.assistant_can_query(query);
                    let expected = self.assistant.generation;
                    let approved = query.clone();
                    conversation = conversation.child(
                        v_flex()
                            .gap_2()
                            .p_2()
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(
                                div().text_sm().child(
                                    language.format(
                                        Message::AssistantQueryTarget,
                                        &[("target", target)],
                                    ),
                                ),
                            )
                            .child(div().text_sm().child(description))
                            .child(div().text_xs().child(t(Message::AssistantQueryPermission)))
                            .when(!allowed, |v| {
                                v.child(
                                    div().text_xs().child(t(Message::AssistantQueryUnavailable)),
                                )
                            })
                            .child(
                                Button::new("assistant-approve-query")
                                    .debug_selector(|| "assistant-approve-query".into())
                                    .label(t(Message::AssistantQueryApprove))
                                    .small()
                                    .disabled(!allowed)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.authorize_assistant_query(
                                            expected,
                                            approved.clone(),
                                            cx,
                                        )
                                    })),
                            ),
                    );
                }
                conversation = conversation.child(
                    Button::new("assistant-copy")
                        .debug_selector(|| "assistant-copy".into())
                        .label(t(if panel.copied {
                            Message::AssistantCopied
                        } else {
                            Message::AssistantCopyAnalysis
                        }))
                        .small()
                        .ghost()
                        .on_click(cx.listener(move |view, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                            view.assistant_copy_feedback(cx);
                        })),
                );
                if let Some(command) = &result.command {
                    conversation = conversation.child(
                        div()
                            .text_sm()
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(command.clone()),
                    );
                    if result.danger {
                        conversation =
                            conversation.child(div().text_sm().child(t(Message::AssistantDanger)));
                    }
                    let copy_command = command.clone();
                    conversation = conversation.child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                Button::new("assistant-copy-command")
                                    .label(t(Message::AssistantCopyCommand))
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_command.clone(),
                                        ));
                                        view.assistant_copy_feedback(cx);
                                    })),
                            )
                            .child(
                                Button::new("assistant-insert")
                                    .debug_selector(|| "assistant-insert".into())
                                    .label(t(Message::AssistantInsert))
                                    .small()
                                    .disabled(!self.assistant_can_insert())
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.insert_assistant_command(window, cx)
                                    })),
                            ),
                    );
                }
            },
        }
        let mut composer =
            v_flex().gap_2().p_3().flex_shrink_0().border_t_1().border_color(cx.theme().border);
        if panel.kind == AnalysisKind::Failure {
            composer = composer.child(div().text_sm().child(
                panel.failure_request.as_ref().map(|r| r.command.clone()).unwrap_or_default(),
            ));
        }
        composer = composer
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(t(
                if panel.kind == AnalysisKind::Chat {
                    Message::AssistantChatHint
                } else {
                    Message::AssistantContextHint
                },
            )))
            .child(
                div()
                    .debug_selector(|| "assistant-context".into())
                    .h(px(88.0))
                    .child(Input::new(&panel.input).h(px(88.0)).disabled(pending)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("assistant-analyze")
                            .debug_selector(|| "assistant-analyze".into())
                            .label(t(if panel.kind == AnalysisKind::Chat {
                                Message::AssistantSend
                            } else {
                                Message::AssistantAnalyze
                            }))
                            .primary()
                            .small()
                            .disabled(
                                pending
                                    || (panel.kind == AnalysisKind::Chat
                                        && panel.input.read(cx).value().trim().is_empty()),
                            )
                            .on_click(
                                cx.listener(|view, _, window, cx| view.send_assistant(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("assistant-cancel")
                            .debug_selector(|| "assistant-cancel".into())
                            .label(t(Message::CommonCancel))
                            .small()
                            .ghost()
                            .disabled(!pending)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.assistant.cancel();
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("assistant-new-chat")
                            .debug_selector(|| "assistant-new-chat".into())
                            .label(t(Message::AssistantNewChat))
                            .small()
                            .ghost()
                            .disabled(pending)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.assistant.history.clear();
                                view.assistant.last_request = None;
                                view.assistant.last_outcome = None;
                                view.open_assistant(AnalysisKind::Chat, None, window, cx);
                            })),
                    ),
            )
            .child(
                Checkbox::new("assistant-auto")
                    .checked(self.assistant.auto)
                    .label(t(Message::AssistantAuto))
                    .on_click(cx.listener(|view, _: &bool, _, cx| view.toggle_assistant_auto(cx))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t(Message::AssistantDataHint)),
            );
        Some(
            v_flex()
                .id("terminal-assistant")
                .debug_selector(|| "terminal-assistant".into())
                .key_context("NebulaTerminalAssistant")
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .bg(cx.theme().background)
                .text_color(cx.theme().foreground)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
                .on_mouse_move(|_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        view.close_assistant(window, cx);
                        cx.stop_propagation();
                    }
                }))
                .child(
                    h_flex()
                        .p_3()
                        .gap_2()
                        .flex_wrap()
                        .flex_shrink_0()
                        .child(div().flex_1().font_semibold().child(t(Message::AssistantTitle)))
                        .child(
                            Button::new("assistant-settings")
                                .debug_selector(|| "assistant-settings".into())
                                .label(t(Message::AssistantSettings))
                                .small()
                                .ghost()
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.emit(TerminalViewEvent::AssistantSettings)
                                })),
                        )
                        .child(
                            Button::new("assistant-close")
                                .debug_selector(|| "assistant-close".into())
                                .icon(IconName::PanelRight)
                                .tooltip(t(Message::AssistantCollapse))
                                .aria_label(t(Message::AssistantCollapse))
                                .small()
                                .ghost()
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.close_assistant(window, cx)
                                })),
                        ),
                )
                .child(
                    h_flex().px_3().pb_2().gap_1().flex_wrap().flex_shrink_0().children(
                        [
                            (AnalysisKind::Chat, Message::AssistantChat),
                            (AnalysisKind::Explain, Message::AssistantExplain),
                            (AnalysisKind::Logs, Message::AssistantLogs),
                            (AnalysisKind::Failure, Message::AssistantFailure),
                        ]
                        .into_iter()
                        .map(|(kind, label)| {
                            Button::new(t(label))
                                .label(t(label))
                                .small()
                                .ghost()
                                .selected(panel.kind == kind)
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.open_assistant(kind, None, window, cx)
                                }))
                        }),
                    ),
                )
                .child(
                    div()
                        .id("assistant-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(&self.assistant.scroll)
                        .p_3()
                        .child(conversation),
                )
                .child(composer)
                .into_any_element(),
        )
    }

    fn assistant_copy_feedback(&mut self, cx: &mut Context<Self>) {
        if let Some(panel) = &mut self.assistant.panel {
            panel.copied = true;
        }
        cx.notify();
    }
}
