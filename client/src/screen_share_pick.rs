//! Browser display capture via `getDisplayMedia`.

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

pub async fn start_display_capture() -> Result<JsValue, String> {
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
