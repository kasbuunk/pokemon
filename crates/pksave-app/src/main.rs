//! pksave-app: egui/eframe GUI for the `pksave` Gen 1 save editor.
//!
//! One codebase, two targets: a native desktop binary and a
//! `wasm32-unknown-unknown` build bootstrapped by trunk (see `index.html`
//! and `Trunk.toml` at the crate root).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod io;
mod screens;
mod widgets;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 740.0])
            .with_title("pksave — Gen 1 save editor"),
        ..Default::default()
    };
    eframe::run_native(
        "pksave",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    console_error_panic_hook::set_once();
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();
    register_beforeunload_guard();

    let web_options = eframe::WebOptions::default();
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("no document");
        let canvas = document
            .get_element_by_id("pksave_canvas")
            .expect("index.html must have a canvas with id pksave_canvas")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("pksave_canvas is not a canvas element");

        let start_result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
            )
            .await;

        if let Err(e) = start_result {
            log::error!("failed to start eframe: {e:?}");
        }
    });
}

/// Register a `beforeunload` listener once; it consults the dirty flag
/// kept in sync by [`app::publish_dirty`] and asks the browser to warn
/// before the tab closes while there are unsaved changes.
#[cfg(target_arch = "wasm32")]
fn register_beforeunload_guard() {
    use eframe::wasm_bindgen::closure::Closure;
    use eframe::wasm_bindgen::JsCast as _;

    let closure = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(
        |event: web_sys::BeforeUnloadEvent| {
            if app::is_dirty_published() {
                event.prevent_default();
                // Legacy API still required by some browsers to show the prompt.
                event.set_return_value("You have unsaved changes.");
            }
        },
    );
    if let Some(window) = web_sys::window() {
        let _ = window
            .add_event_listener_with_callback("beforeunload", closure.as_ref().unchecked_ref());
    }
    closure.forget();
}
