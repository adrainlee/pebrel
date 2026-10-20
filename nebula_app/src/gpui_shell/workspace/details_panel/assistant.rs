//! Workspace geometry and navigation; conversations remain owned by their panes.
use super::*;
use crate::gpui_shell::terminal::view::AssistantPanel;

#[cfg(all(test, feature = "gpui-test-support"))]
mod tests;

#[derive(Default)]
pub(super) struct AssistantDock {
    pub(super) open: bool,
    panel: Option<(gpui::EntityId, Entity<AssistantPanel>)>,
}

impl NebulaWorkspace {
    pub(in crate::gpui_shell::workspace) fn assistant_dock_visible(&self) -> bool {
        self.details_panel.assistant.open
            && !self.settings_open
            && self.tabs.get(self.active).and_then(WorkspaceTab::focused_view).is_some()
    }

    pub(in crate::gpui_shell::workspace) fn set_assistant_dock(
        &mut self,
        open: bool,
        cx: &mut Context<Self>,
    ) {
        self.details_panel.assistant.open = open;
        self.cancel_details_panel_resize(cx);
        cx.notify();
    }

    pub(in crate::gpui_shell::workspace) fn toggle_assistant_dock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.assistant_dock_visible() {
            if let Some(terminal) =
                self.tabs.get(self.active).and_then(WorkspaceTab::focused_view).cloned()
            {
                terminal.update(cx, |view, cx| view.close_assistant(window, cx));
            }
            self.set_assistant_dock(false, cx);
            self.focus_active(window, cx);
        } else if let Some(terminal) =
            self.tabs.get(self.active).and_then(WorkspaceTab::focused_view).cloned()
        {
            self.set_assistant_dock(true, cx);
            terminal.update(cx, |view, cx| view.open_assistant_default(window, cx));
        }
    }

    pub(in crate::gpui_shell::workspace) fn render_assistant_dock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let terminal = self.tabs[self.active].focused_view().unwrap().clone();
        let dock = &mut self.details_panel.assistant;
        if dock.panel.as_ref().is_none_or(|(id, _)| *id != terminal.entity_id()) {
            let panel = cx.new(|cx| AssistantPanel::new(&terminal, window, cx));
            dock.panel = Some((terminal.entity_id(), panel));
        }
        // Reserve a real column; it never covers the terminal grid. Narrow
        // windows keep a terminal area, while the assistant scrolls vertically.
        let width = (f32::from(window.viewport_size().width) * 0.46).clamp(180.0, 400.0);
        div()
            .id("workspace-assistant-dock")
            .debug_selector(|| "workspace-assistant-dock".into())
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(dock.panel.as_ref().unwrap().1.clone())
            .into_any_element()
    }

    pub(in crate::gpui_shell::workspace) fn open_assistant_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings(window, cx);
        if let Some((settings, _)) = &self.settings_surface {
            settings.update(cx, |settings, cx| settings.show_providers(window, cx));
        }
    }
}
