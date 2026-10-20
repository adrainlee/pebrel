use super::*;

struct Providers(crate::ai_providers::ProviderStore);

impl Providers {
    fn install() -> Self {
        use crate::ai_providers::{AiProvider, ProviderKind};
        let previous = crate::ai_providers::load();
        let mut store = previous.clone();
        for provider in &mut store.providers {
            provider.enabled = false;
        }
        for (id, model) in [("fixture-first", "model-one"), ("fixture-second", "model-two")] {
            let mut provider = AiProvider::preset(ProviderKind::Custom, id);
            provider.name = id.into();
            provider.base_url = "http://127.0.0.1:1/v1".into();
            provider.model = model.into();
            provider.api_key_set = true;
            store.providers.push(provider);
        }
        store.active_id = "fixture-first".into();
        crate::ai_providers::save(&store).unwrap();
        Self(previous)
    }
}

impl Drop for Providers {
    fn drop(&mut self) {
        crate::ai_providers::save(&self.0).unwrap();
    }
}

#[gpui::test]
fn visible_model_picker_routes_new_requests_and_revokes_old_command_confirmation(
    cx: &mut TestAppContext,
) {
    let _settings = crate::gpui_shell::settings_fixture::lock_theme_studio();
    let _providers = Providers::install();
    let (view, mut cx, receiver) = open(cx);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            feed(view, b"\x1b]133;A\x07$ \x1b]133;B\x07");
            view.assistant.worker = Some(Arc::new(move |request| {
                recorded.lock().unwrap().push(request.clone());
                Ok(advice())
            }));
            view.open_assistant(
                AnalysisKind::Command,
                Some("Build this project".into()),
                window,
                cx,
            );
        })
    });
    draw(&mut cx);
    click("assistant-analyze", &mut cx);
    assert_eq!(requests.lock().unwrap()[0].provider.as_ref().unwrap().model, "model-one");
    assert!(view.read_with(&cx, |view, cx| view.assistant_can_execute(cx)));
    let old_generation = view.read_with(&cx, |view, _| view.assistant.generation);
    click("assistant-model", &mut cx);
    cx.simulate_keystrokes("down down enter");
    draw(&mut cx);
    assert_eq!(
        view.read_with(&cx, |view, _| view.assistant.models.provider().unwrap().model.clone()),
        "model-two"
    );
    assert_eq!(requests.lock().unwrap().len(), 1, "model selection does not submit a request");
    view.update(&mut cx, |view, cx| {
        view.execute_assistant_command(old_generation, "cargo build", cx);
        view.finish_assistant(old_generation, view.prompt_input_epoch, false, Ok(advice()), cx);
        assert!(!view.assistant_can_execute(cx));
    });
    assert!(input(&receiver).is_empty());
    click("assistant-analyze", &mut cx);
    assert_eq!(requests.lock().unwrap()[1].provider.as_ref().unwrap().model, "model-two");
    assert_eq!(
        crate::ai_providers::load().active_id,
        "fixture-first",
        "pane choice does not change global defaults"
    );
    click("assistant-close", &mut cx);
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_assistant_default(window, cx)));
    draw(&mut cx);
    assert_eq!(
        view.read_with(&cx, |view, _| view.assistant.models.provider().unwrap().model.clone()),
        "model-two"
    );
    click("assistant-analyze", &mut cx);
    click("assistant-execute", &mut cx);
    assert_eq!(input(&receiver), b"cargo build");
    view.update(&mut cx, |view, cx| {
        view.select_assistant_model("fixture-first", cx);
        feed(view, b"cargo build");
    });
    cx.executor().advance_clock(std::time::Duration::from_millis(25));
    draw(&mut cx);
    assert!(input(&receiver).is_empty(), "switching also cancels a pending Enter");
    let mut store = crate::ai_providers::load();
    let provider =
        store.providers.iter_mut().find(|provider| provider.id == "fixture-first").unwrap();
    provider.name = "Long provider name ".repeat(12);
    provider.model = "long-model-name-".repeat(12);
    crate::ai_providers::save(&store).unwrap();
    cx.update(|window, cx| view.update(cx, |view, cx| view.open_assistant_default(window, cx)));
    cx.simulate_resize(gpui::size(px(300.0), px(600.0)));
    draw(&mut cx);
    let picker = cx.debug_bounds("assistant-model").unwrap();
    let panel = cx.debug_bounds("terminal-assistant").unwrap();
    assert!(
        picker.left() >= panel.left() && picker.right() <= panel.right(),
        "long model names must fit the narrow panel"
    );
}

#[gpui::test]
fn unavailable_selected_model_never_falls_back_to_another_provider(cx: &mut TestAppContext) {
    let _settings = crate::gpui_shell::settings_fixture::lock_theme_studio();
    let _providers = Providers::install();
    let (view, mut cx, receiver) = open(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_assistant(
                AnalysisKind::Chat,
                Some("Explain this command".into()),
                window,
                cx,
            );
            view.select_assistant_model("fixture-second", cx);
            view.assistant.worker = Some(Arc::new(|_| panic!("unavailable model must not send")));
        })
    });
    let mut store = crate::ai_providers::load();
    store.providers.iter_mut().find(|provider| provider.id == "fixture-second").unwrap().enabled =
        false;
    crate::ai_providers::save(&store).unwrap();
    draw(&mut cx);
    click("assistant-analyze", &mut cx);
    assert!(view.read_with(&cx, |view, _| matches!(
        view.assistant.panel.as_ref().unwrap().status,
        Status::Error(AnalysisError::NoProvider)
    )));
    assert!(input(&receiver).is_empty());
}
