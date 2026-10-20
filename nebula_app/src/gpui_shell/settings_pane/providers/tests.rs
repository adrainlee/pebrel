use super::*;

#[gpui::test]
fn command_prediction_switch_is_visible_persists_and_invalidates_candidates(
    cx: &mut gpui::TestAppContext,
) {
    let _lock = crate::gpui_shell::settings_fixture::lock_theme_studio();
    struct Restore(std::path::PathBuf, Vec<u8>);
    impl Drop for Restore {
        fn drop(&mut self) {
            std::fs::write(&self.0, &self.1).unwrap();
        }
    }
    let path = crate::ai_providers::store_path();
    let original = std::fs::read(&path)
        .unwrap_or_else(|_| serde_json::to_vec_pretty(&crate::ai_providers::load()).unwrap());
    let _restore = Restore(path.clone(), original);
    let provider = crate::ai_providers::AiProvider::preset(
        crate::ai_providers::ProviderKind::Custom,
        "prediction-settings-fixture",
    );
    crate::ai_providers::save(&crate::ai_providers::ProviderStore {
        active_id: provider.id.clone(),
        providers: vec![provider],
    })
    .unwrap();
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(crate::gpui_shell::config::Settings::load(nebula_settings::ThemeName::Nord));
    });
    let mut pane = None;
    let (_, window) = cx.add_window_view(|window, cx| {
        let entity = cx.new(|cx| SettingsPane::new(window, cx));
        entity.update(cx, |pane, cx| pane.show_providers(window, cx));
        pane = Some(entity.clone());
        gpui_component::Root::new(entity, window, cx)
    });
    let pane = pane.unwrap();
    window.simulate_resize(gpui::size(px(1280.0), px(900.0)));
    for enabled in [true, false] {
        window.run_until_parked();
        window.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let bounds = window
            .debug_bounds("nebula-switch-provider-command-completion")
            .expect("visible AI completion switch");
        assert!(bounds.origin.y >= px(0.0) && bounds.bottom() <= px(900.0));
        let generation = crate::ai_providers::config_generation();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        window.run_until_parked();
        assert_eq!(
            crate::ai_providers::active_enabled(&crate::ai_providers::load())
                .unwrap()
                .command_completion,
            enabled
        );
        assert!(crate::ai_providers::config_generation() > generation);
        assert!(pane.read_with(window, |pane, _| matches!(
            pane.provider_status,
            Some(ProviderStatus::Saved)
        )));
    }
}
