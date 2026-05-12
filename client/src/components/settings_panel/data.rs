//! Browser device data for settings panel UI.

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use web_sys::{MediaStream, MediaStreamConstraints, MediaStreamTrack};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Input,
    Output,
    Camera,
}

impl DeviceKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Input => "Microphone",
            Self::Output => "Speakers",
            Self::Camera => "Camera",
        }
    }

    pub fn nav_label(self) -> &'static str {
        match self {
            Self::Input => "INPUT",
            Self::Output => "OUTPUT",
            Self::Camera => "CAMERA",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DeviceEntry {
    pub id: String,
    pub label: String,
}

/// All device groups used by [`crate::components::SettingsPanel`].
#[derive(Clone, Default, PartialEq, Eq)]
pub struct DeviceCatalog {
    pub inputs: Vec<DeviceEntry>,
    pub outputs: Vec<DeviceEntry>,
    pub cameras: Vec<DeviceEntry>,
}

pub async fn load_device_catalog() -> Result<DeviceCatalog, String> {
    let window = web_sys::window().ok_or_else(|| "Window is not available.".to_owned())?;
    let navigator = window.navigator();
    let media_devices = js_sys::Reflect::get(&navigator, &JsValue::from_str("mediaDevices"))
        .map_err(js_error_to_string)?;
    if media_devices.is_undefined() || media_devices.is_null() {
        return Err("Media devices are not supported by this browser.".to_owned());
    }

    // Browsers often omit usable `deviceId` values (or omit devices entirely) until the
    // user has granted capture permission at least once.
    prime_enumeration_permissions().await;

    let enumerate_devices =
        js_sys::Reflect::get(&media_devices, &JsValue::from_str("enumerateDevices"))
            .map_err(js_error_to_string)?
            .dyn_into::<js_sys::Function>()
            .map_err(|_| "enumerateDevices is not available.".to_owned())?;
    let promise = enumerate_devices
        .call0(&media_devices)
        .map_err(js_error_to_string)?
        .dyn_into::<js_sys::Promise>()
        .map_err(|_| "enumerateDevices did not return a Promise.".to_owned())?;
    let devices = JsFuture::from(promise).await.map_err(js_error_to_string)?;
    let devices = js_sys::Array::from(&devices);

    let mut catalog = DeviceCatalog::default();
    for value in devices.iter() {
        let kind = get_string_property(&value, "kind");
        let id = get_string_property(&value, "deviceId");
        if id.is_empty() {
            continue;
        }
        let label = get_string_property(&value, "label");
        let label = if label.is_empty() {
            default_label(&kind, catalog_len_for_kind(&catalog, &kind) + 1)
        } else {
            label
        };
        let entry = DeviceEntry { id, label };
        match kind.as_str() {
            "audioinput" => catalog.inputs.push(entry),
            "audiooutput" => catalog.outputs.push(entry),
            "videoinput" => catalog.cameras.push(entry),
            _ => {}
        }
    }

    Ok(catalog)
}

/// Best-effort `getUserMedia` so a follow-up `enumerateDevices` returns real ids/labels.
async fn prime_enumeration_permissions() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(devices) = window.navigator().media_devices() else {
        return;
    };

    for (audio, video) in [(true, true), (true, false), (false, true)] {
        if !audio && !video {
            continue;
        }
        if try_prime_media(&devices, audio, video).await.is_ok() {
            return;
        }
    }
}

async fn try_prime_media(
    devices: &web_sys::MediaDevices,
    audio: bool,
    video: bool,
) -> Result<(), JsValue> {
    let constraints = MediaStreamConstraints::new();
    constraints.set_audio(&JsValue::from(audio));
    constraints.set_video(&JsValue::from(video));
    let promise = devices.get_user_media_with_constraints(&constraints)?;
    let stream = JsFuture::from(promise).await?;
    let stream: MediaStream = stream.dyn_into()?;
    for track in stream.get_tracks().iter() {
        if let Ok(track) = track.dyn_into::<MediaStreamTrack>() {
            track.stop();
        }
    }
    Ok(())
}

fn catalog_len_for_kind(catalog: &DeviceCatalog, kind: &str) -> usize {
    match kind {
        "audioinput" => catalog.inputs.len(),
        "audiooutput" => catalog.outputs.len(),
        "videoinput" => catalog.cameras.len(),
        _ => 0,
    }
}

fn default_label(kind: &str, index: usize) -> String {
    match kind {
        "audioinput" => format!("Microphone {index}"),
        "audiooutput" => format!("Speaker {index}"),
        "videoinput" => format!("Camera {index}"),
        _ => format!("Device {index}"),
    }
}

fn get_string_property(value: &JsValue, property: &str) -> String {
    js_sys::Reflect::get(value, &JsValue::from_str(property))
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default()
}

fn js_error_to_string(value: JsValue) -> String {
    value.as_string().unwrap_or_else(|| {
        js_sys::JSON::stringify(&value)
            .ok()
            .and_then(|s| s.as_string())
            .unwrap_or_else(|| "Browser device operation failed.".to_owned())
    })
}
