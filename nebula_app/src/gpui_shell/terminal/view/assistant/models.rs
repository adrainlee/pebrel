//! Per-pane model choice; provider settings and prediction defaults stay shared.
use super::*;
use crate::ai_providers::{AiProvider, ProviderStore};
use crate::gpui_shell::prelude::*;
use crate::i18n::Message;
use gpui_component::menu::PopupMenuItem;

#[derive(Default)]
pub(super) struct Selection {
    store: ProviderStore,
    selected: Option<String>,
}

fn available(provider: &AiProvider) -> bool {
    provider.enabled
        && !provider.model.trim().is_empty()
        && (provider.base_url.starts_with("https://") || provider.base_url.starts_with("http://"))
        && (!provider.kind.requires_api_key() || provider.api_key_set)
}

impl Selection {
    pub(super) fn refresh(&mut self) {
        self.store = crate::ai_providers::load();
        if self.selected.is_none() {
            self.selected = self
                .store
                .providers
                .iter()
                .find(|provider| provider.id == self.store.active_id && available(provider))
                .or_else(|| self.store.providers.iter().find(|provider| available(provider)))
                .map(|provider| provider.id.clone());
        }
    }

    pub(super) fn provider(&self) -> Option<&AiProvider> {
        let id = self.selected.as_ref()?;
        self.store.providers.iter().find(|provider| &provider.id == id)
    }

    pub(super) fn request_provider(&mut self) -> Result<Option<AiProvider>, AnalysisError> {
        self.refresh();
        if self.selected.is_some() && self.provider().is_none_or(|provider| !available(provider)) {
            return Err(AnalysisError::NoProvider);
        }
        Ok(self.provider().cloned())
    }
}

impl TerminalView {
    pub(super) fn select_assistant_model(&mut self, id: &str, cx: &mut Context<Self>) {
        self.assistant.models.refresh();
        if self.assistant.models.selected.as_deref() == Some(id)
            || !self
                .assistant
                .models
                .store
                .providers
                .iter()
                .any(|provider| provider.id == id && available(provider))
        {
            return;
        }
        self.assistant.cancel();
        self.assistant.models.selected = Some(id.to_owned());
        self.assistant.last_request = None;
        self.assistant.last_outcome = None;
        self.assistant.auto_request = None;
        self.assistant.auto_outcome = None;
        self.assistant.last_completed = None;
        if let Some(panel) = &mut self.assistant.panel {
            panel.status = Status::Idle;
            panel.copied = false;
        }
        cx.notify();
    }

    pub(super) fn assistant_model_picker(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let language = crate::gpui_shell::config::ui_language(cx);
        let label = self
            .assistant
            .models
            .provider()
            .map(|provider| format!("{} · {}", provider.name, provider.model))
            .unwrap_or_else(|| language.text(Message::AssistantSelectModel).to_owned());
        let owner = cx.weak_entity();
        let description = format!("{}: {label}", language.text(Message::AssistantSelectModel));
        Button::new("assistant-model")
            .debug_selector(|| "assistant-model".into())
            .w_full()
            .child(div().flex_1().min_w_0().truncate().text_left().child(label))
            .tooltip(description.clone())
            .aria_label(description)
            .small()
            .ghost()
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, cx| {
                let choices = owner
                    .update(cx, |view, _| {
                        view.assistant.models.refresh();
                        view.assistant
                            .models
                            .store
                            .providers
                            .iter()
                            .filter(|provider| available(provider))
                            .map(|provider| {
                                (
                                    provider.clone(),
                                    view.assistant.models.selected.as_deref() == Some(&provider.id),
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if choices.is_empty() {
                    return menu.item(
                        PopupMenuItem::new(language.text(Message::AssistantNoProvider))
                            .disabled(true),
                    );
                }
                for (provider, selected) in choices {
                    let owner = owner.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("{} · {}", provider.name, provider.model))
                            .checked(selected)
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |view, cx| {
                                    view.select_assistant_model(&provider.id, cx)
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }
}
