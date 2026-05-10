//! Screen-share source enumeration and system capture. UI lives in [`crate::components::ScreenShareDialog`].

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

/// One selectable **display** (monitor) in the screen-share dialog — not application windows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenShareSource {
    pub id: String,
    pub label: String,
    pub width_px: u32,
    pub height_px: u32,
}

/// Browsers do not expose monitor enumeration consistently before `getDisplayMedia`.
/// Render one generic entry and let the native picker choose the actual display.
pub async fn enumerate_screen_share_sources_for_dialog() -> Vec<ScreenShareSource> {
    gloo_timers::future::TimeoutFuture::new(0).await;
    vec![ScreenShareSource {
        id: "browser-display-picker".to_string(),
        label: "Choose a display".to_string(),
        width_px: 16,
        height_px: 9,
    }]
}

/// Start sharing the surface chosen in the dialog using the OS/browser capture path.
pub async fn start_system_screen_share_for_stream(source_id: &str) -> Result<JsValue, String> {
    let _ = source_id;
    let window = web_sys::window().ok_or_else(|| "Window is not available.".to_owned())?;
    let navigator = window.navigator();
    let media_devices = js_sys::Reflect::get(&navigator, &JsValue::from_str("mediaDevices"))
        .map_err(js_error_to_string)?;
    if media_devices.is_undefined() || media_devices.is_null() {
        return Err("Screen sharing is not supported by this browser.".to_owned());
    }

    let get_display_media =
        js_sys::Reflect::get(&media_devices, &JsValue::from_str("getDisplayMedia"))
            .map_err(js_error_to_string)?
            .dyn_into::<js_sys::Function>()
            .map_err(|_| "getDisplayMedia is not available.".to_owned())?;

    let constraints = js_sys::Object::new();
    js_sys::Reflect::set(&constraints, &JsValue::from_str("video"), &JsValue::TRUE)
        .map_err(js_error_to_string)?;
    js_sys::Reflect::set(&constraints, &JsValue::from_str("audio"), &JsValue::FALSE)
        .map_err(js_error_to_string)?;

    let promise = get_display_media
        .call1(&media_devices, &constraints)
        .map_err(js_error_to_string)?
        .dyn_into::<js_sys::Promise>()
        .map_err(|_| "getDisplayMedia did not return a Promise.".to_owned())?;

    JsFuture::from(promise).await.map_err(js_error_to_string)
}

pub fn stop_media_stream_tracks(stream: &JsValue) {
    let Ok(media_stream) = stream.clone().dyn_into::<web_sys::MediaStream>() else {
        return;
    };
    let tracks = media_stream.get_tracks();
    for track in tracks.iter() {
        if let Ok(track) = track.dyn_into::<web_sys::MediaStreamTrack>() {
            track.stop();
        }
    }
}

fn js_error_to_string(value: JsValue) -> String {
    value.as_string().unwrap_or_else(|| {
        js_sys::JSON::stringify(&value)
            .ok()
            .and_then(|s| s.as_string())
            .unwrap_or_else(|| "Browser media operation failed.".to_owned())
    })
}
