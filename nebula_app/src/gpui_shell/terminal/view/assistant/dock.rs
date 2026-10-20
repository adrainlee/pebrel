//! A workspace-mounted surface observing the pane-owned conversation.
use super::*;
use crate::gpui_shell::prelude::*;

pub(in crate::gpui_shell) struct AssistantPanel {
    terminal: gpui::WeakEntity<TerminalView>,
    _subscription: gpui::Subscription,
}

impl AssistantPanel {
    pub(in crate::gpui_shell) fn new(
        terminal: &gpui::Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        terminal.update(cx, |view, cx| {
            if view.assistant.panel.is_none() {
                view.prepare_assistant(AnalysisKind::Chat, None, window, cx);
            }
        });
        Self {
            terminal: terminal.downgrade(),
            _subscription: cx.observe(terminal, |_, _, cx| cx.notify()),
        }
    }
}

impl Render for AssistantPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().min_w_0().min_h_0().children(
            self.terminal
                .upgrade()
                .and_then(|terminal| terminal.update(cx, |view, cx| view.assistant_chrome(cx))),
        )
    }
}
