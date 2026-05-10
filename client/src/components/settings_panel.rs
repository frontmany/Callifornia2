//! Global settings panel.

#[path = "settings_panel/data.rs"]
mod data;

use crate::components::ThemeToggle;
use crate::theme::Theme;
use data::{DeviceCatalog, DeviceEntry, DeviceKind, load_device_catalog};
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlInputElement;
use yew::prelude::*;

const MIN_LEVEL: u32 = 0;
const MAX_LEVEL: u32 = 200;
const DEFAULT_LEVEL: u32 = 100;

#[derive(Properties, PartialEq)]
pub struct SettingsPanelProps {
    pub on_close: Callback<()>,
    pub theme: Theme,
    pub on_theme_change: Callback<Theme>,
    pub mic_enabled: bool,
    pub camera_enabled: bool,
    pub input_level: u32,
    pub output_level: u32,
    pub on_toggle_mic: Callback<()>,
    pub on_toggle_camera: Callback<()>,
    pub on_input_level_change: Callback<u32>,
    pub on_output_level_change: Callback<u32>,
    pub selected_mic_device_id: String,
    pub selected_speaker_device_id: String,
    pub selected_camera_device_id: String,
    pub on_mic_device_change: Callback<String>,
    pub on_speaker_device_change: Callback<String>,
    pub on_camera_device_change: Callback<String>,
}

#[function_component]
pub fn SettingsPanel(props: &SettingsPanelProps) -> Html {
    let tab = use_state(|| DeviceKind::Input);
    let catalog = use_state(DeviceCatalog::default);
    let catalog_error = use_state(|| Option::<String>::None);

    {
        let catalog = catalog.clone();
        let catalog_error = catalog_error.clone();
        use_effect_with((), move |_| {
            spawn_local(async move {
                match load_device_catalog().await {
                    Ok(next) => {
                        catalog.set(next);
                        catalog_error.set(None);
                    }
                    Err(err) => catalog_error.set(Some(err)),
                }
            });
            || ()
        });
    }

    let on_input_level = on_level_input(props.on_input_level_change.clone());
    let on_output_level = on_level_input(props.on_output_level_change.clone());

    let current_list = match *tab {
        DeviceKind::Input => render_device_list(
            catalog.inputs.as_slice(),
            &props.selected_mic_device_id,
            props.on_mic_device_change.clone(),
        ),
        DeviceKind::Output => render_device_list(
            catalog.outputs.as_slice(),
            &props.selected_speaker_device_id,
            props.on_speaker_device_change.clone(),
        ),
        DeviceKind::Camera => render_device_list(
            catalog.cameras.as_slice(),
            &props.selected_camera_device_id,
            props.on_camera_device_change.clone(),
        ),
    };

    html! {
        <div class="settings-panel" role="presentation">
            <button
                type="button"
                class="settings-panel__backdrop"
                aria-hidden="true"
                tabindex="-1"
                onclick={props.on_close.reform(|_| ())}
            />

            <div
                class="settings-panel__panel font-manrope text-on-background"
                role="dialog"
                aria-modal="true"
                aria-labelledby="settings-panel-title"
            >
                <nav class="settings-panel__nav" aria-label="Settings sections">
                    <h2 class="settings-panel__nav-title">{ "Settings" }</h2>
                    { tab_button(DeviceKind::Input, tab.clone()) }
                    { tab_button(DeviceKind::Output, tab.clone()) }
                    { tab_button(DeviceKind::Camera, tab.clone()) }

                    <div class="settings-panel__theme-switch">
                        <span class="settings-panel__theme-label">{ "Theme" }</span>
                        <ThemeToggle theme={props.theme} on_change={props.on_theme_change.clone()} />
                    </div>

                    <div class="settings-panel__quick-toggles" aria-label="Quick media toggles">
                        <button
                            type="button"
                            class={classes!(
                                "settings-panel__quick-btn",
                                if props.mic_enabled {
                                    Some("settings-panel__quick-btn--on")
                                } else {
                                    Some("settings-panel__quick-btn--off")
                                }
                            )}
                            onclick={props.on_toggle_mic.reform(|_| ())}
                            aria-label={if props.mic_enabled { "Turn microphone off" } else { "Turn microphone on" }}
                            title={if props.mic_enabled { "Microphone on" } else { "Microphone off" }}
                        >
                            <img
                                class="settings-panel__quick-icon"
                                src={if props.mic_enabled {
                                    "icons/microphone.svg"
                                } else {
                                    "icons/mute-enabled-microphone.svg"
                                }}
                                alt=""
                                aria-hidden="true"
                            />
                        </button>

                        <button
                            type="button"
                            class={classes!(
                                "settings-panel__quick-btn",
                                if props.camera_enabled {
                                    Some("settings-panel__quick-btn--on")
                                } else {
                                    Some("settings-panel__quick-btn--off")
                                }
                            )}
                            onclick={props.on_toggle_camera.reform(|_| ())}
                            aria-label={if props.camera_enabled { "Turn camera off" } else { "Turn camera on" }}
                            title={if props.camera_enabled { "Camera on" } else { "Camera off" }}
                        >
                            <img
                                class="settings-panel__quick-icon"
                                src={if props.camera_enabled {
                                    "icons/camera.svg"
                                } else {
                                    "icons/cameraDisabled.svg"
                                }}
                                alt=""
                                aria-hidden="true"
                            />
                        </button>
                    </div>
                </nav>

                <section class="settings-panel__main">
                    <header class="settings-panel__header">
                        <h3 id="settings-panel-title" class="settings-panel__section-title">{ (*tab).title() }</h3>
                        <button
                            type="button"
                            class="settings-panel__close"
                            aria-label="Close"
                            onclick={props.on_close.reform(|_| ())}
                        >
                            {"×"}
                        </button>
                    </header>

                    { current_list }

                    if let Some(message) = &*catalog_error {
                        <p class="body-md" role="alert">{ message }</p>
                    }

                    if matches!(*tab, DeviceKind::Input) {
                        <div class="settings-panel__slider-row">
                            { level_slider(props.input_level, on_input_level) }
                        </div>
                    }

                    if matches!(*tab, DeviceKind::Output) {
                        <div class="settings-panel__slider-row">
                            { level_slider(props.output_level, on_output_level) }
                        </div>
                    }
                </section>
            </div>
        </div>
    }
}

fn tab_button(tab_kind: DeviceKind, selected: UseStateHandle<DeviceKind>) -> Html {
    let is_active = *selected == tab_kind;
    let class_name = classes!(
        "settings-panel__nav-btn",
        is_active.then_some("settings-panel__nav-btn--active")
    );
    let onclick = {
        let selected = selected.clone();
        Callback::from(move |_| selected.set(tab_kind))
    };

    html! {
        <button type="button" class={class_name} onclick={onclick}>
            { tab_kind.nav_label() }
        </button>
    }
}

fn render_device_list(
    devices: &[DeviceEntry],
    selected_device_id: &str,
    on_select_device: Callback<String>,
) -> Html {
    html! {
        <div class="settings-panel__list">
            if devices.is_empty() {
                <p class="body-md">{ "No devices found." }</p>
            } else {
                <>
                    { for devices.iter().map(|entry| {
                        let is_selected = selected_device_id == entry.id;
                        let selected_class = is_selected.then_some("settings-panel__item--selected");
                        let onclick = {
                            let on_select_device = on_select_device.clone();
                            let id = entry.id.clone();
                            Callback::from(move |_| on_select_device.emit(id.clone()))
                        };
                        html! {
                            <button
                                type="button"
                                class={classes!("settings-panel__item", selected_class)}
                                onclick={onclick}
                            >
                                <span>{ entry.label.clone() }</span>
                            </button>
                        }
                    }) }
                </>
            }
        </div>
    }
}

fn on_level_input(on_change: Callback<u32>) -> Callback<InputEvent> {
    Callback::from(move |event: InputEvent| {
        let parsed = event
            .target_unchecked_into::<HtmlInputElement>()
            .value()
            .parse::<u32>()
            .unwrap_or(DEFAULT_LEVEL)
            .clamp(MIN_LEVEL, MAX_LEVEL);
        on_change.emit(parsed);
    })
}

fn level_slider(value: u32, oninput: Callback<InputEvent>) -> Html {
    let pct = (value as f32 / MAX_LEVEL as f32) * 100.0;
    html! {
        <input
            class="settings-panel__slider"
            type="range"
            min={MIN_LEVEL.to_string()}
            max={MAX_LEVEL.to_string()}
            value={value.to_string()}
            style={format!("--pct: {:.2}%;", pct)}
            oninput={oninput}
        />
    }
}
