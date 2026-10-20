use super::*;
use crate::gpui_shell::terminal::view::TerminalLaunch;
use gpui::{Modifiers, TestAppContext, VisualTestContext};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let bounds = cx.debug_bounds(selector).expect("visible workspace control");
    cx.simulate_click(bounds.center(), Modifiers::default());
    draw(cx);
}

#[gpui::test]
fn visible_entry_opens_a_right_column_and_model_settings_in_both_tab_layouts(
    cx: &mut TestAppContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::gpui_shell::init(cx, None);
        windowing::initialize(cx, crate::runtime_api::RuntimeHub::new());
        cx.set_reduce_motion(true);
    });
    let mut workspace = None;
    let (_, window) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| {
            let mut workspace = NebulaWorkspace::new(
                window,
                None,
                None,
                1,
                crate::runtime_api::RuntimeHub::new(),
                windowing::WorkspaceStartup::Empty,
                windowing::WindowRole::Regular,
                cx,
            );
            for _ in 0..2 {
                let pane = workspace.new_pane(
                    (80, 24),
                    TerminalLaunch::Local {
                        cwd: None,
                        shell: Some(nebula_terminal::tty::Shell::new(
                            "pebrel-test-missing-shell-executable".into(),
                            vec![],
                        )),
                        shell_name: None,
                    },
                    None,
                    window,
                    cx,
                );
                let focused = pane.id;
                workspace.tabs.push(WorkspaceTab::Terminal {
                    panes: vec![pane],
                    tree: SplitTree::Leaf(focused),
                    focused,
                    zoomed: false,
                    broadcast: false,
                });
                workspace.tab_meta.push(TabMeta::default());
            }
            workspace
        });
        workspace = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let mut cx = window.clone();
    let workspace = workspace.unwrap();
    cx.simulate_resize(gpui::size(px(1200.0), px(850.0)));
    for position in
        [nebula_settings::TabsPositionName::Sidebar, nebula_settings::TabsPositionName::Top]
    {
        workspace.update(&mut cx, |workspace, cx| {
            workspace.tabs_position = position;
            cx.notify();
        });
        draw(&mut cx);
        let entry = cx.debug_bounds("terminal-assistant-toggle").unwrap();
        assert!(entry.size.width > px(60.0), "the entry must show text, not only an icon");
        click("terminal-assistant-toggle", &mut cx);
        let dock = cx.debug_bounds("workspace-assistant-dock").unwrap();
        assert!(dock.left() >= px(790.0));
        assert!(dock.right() <= px(1200.0));
        let composer = cx.debug_bounds("assistant-context").unwrap();
        assert!(composer.left() >= dock.left() && composer.right() <= dock.right());
        assert!(cx.debug_bounds("assistant-settings").is_some());
        let first = workspace.read_with(&cx, |workspace, _| {
            workspace.details_panel.assistant.panel.as_ref().unwrap().0
        });
        workspace.update(&mut cx, |workspace, cx| {
            workspace.active = 1;
            cx.notify();
        });
        draw(&mut cx);
        let second = workspace.read_with(&cx, |workspace, _| {
            workspace.details_panel.assistant.panel.as_ref().unwrap().0
        });
        assert_ne!(first, second, "the dock must follow the focused terminal");
        workspace.update(&mut cx, |workspace, cx| {
            workspace.active = 0;
            cx.notify();
        });
        draw(&mut cx);
        click("assistant-settings", &mut cx);
        assert!(workspace.read_with(&cx, |workspace, _| workspace.settings_open));
        assert!(
            cx.debug_bounds("assistant-provider-settings").is_some(),
            "settings must land on the actual provider editor"
        );
        cx.update(|window, cx| {
            workspace.update(cx, |workspace, cx| workspace.close_settings(window, cx))
        });
        draw(&mut cx);
        assert!(cx.debug_bounds("workspace-assistant-dock").is_some());
        click("assistant-close", &mut cx);
        assert!(cx.debug_bounds("workspace-assistant-dock").is_none());
    }
}
