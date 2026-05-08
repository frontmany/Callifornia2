//! Left-side pre-join panel for media device choices.

#[path = "device_settings_left_panel/data.rs"]
mod data;

use data::{stub_device_catalog, DeviceEntry, DeviceKind};
use web_sys::HtmlInputElement;
use yew::prelude::*;

const MIN_LEVEL: u32 = 0;
const MAX_LEVEL: u32 = 200;
const DEFAULT_LEVEL: u32 = 100;

#[derive(Properties, PartialEq)]
pub struct DeviceSettingsLeftPanelProps {
    pub on_close: Callback<()>,
    pub mic_enabled: bool,
    pub camera_enabled: bool,
    pub input_level: u32,
    pub output_level: u32,
    pub on_toggle_mic: Callback<()>,
    pub on_toggle_camera: Callback<()>,
    pub on_input_level_change: Callback<u32>,
    pub on_output_level_change: Callback<u32>,
}

#[function_component]
pub fn DeviceSettingsLeftPanel(props: &DeviceSettingsLeftPanelProps) -> Html {
    let tab = use_state(|| DeviceKind::Input);

    // NOTE: Device source is currently static (`stub_device_catalog`).
    let catalog = stub_device_catalog();

    let input_selected = use_state(|| catalog.inputs[0].id);
    let output_selected = use_state(|| catalog.outputs[0].id);
    let camera_selected = use_state(|| catalog.cameras[0].id);

    let on_input_level = on_level_input(props.on_input_level_change.clone());
    let on_output_level = on_level_input(props.on_output_level_change.clone());

    let current_list = match *tab {
        DeviceKind::Input => render_device_list(catalog.inputs, input_selected.clone()),
        DeviceKind::Output => render_device_list(catalog.outputs, output_selected.clone()),
        DeviceKind::Camera => render_device_list(catalog.cameras, camera_selected.clone()),
    };

    html! {
        <div class="device-settings-left-panel" role="presentation">
            <button
                type="button"
                class="device-settings-left-panel__backdrop"
                aria-hidden="true"
                tabindex="-1"
                onclick={props.on_close.reform(|_| ())}
            />

            <div
                class="device-settings-left-panel__panel font-manrope text-on-background"
                role="dialog"
                aria-modal="true"
                aria-labelledby="device-settings-title"
            >
                <nav class="device-settings-left-panel__nav" aria-label="Device settings sections">
                    <h2 class="device-settings-left-panel__nav-title">{ "Device Center" }</h2>
                    { tab_button(DeviceKind::Input, tab.clone()) }
                    { tab_button(DeviceKind::Output, tab.clone()) }
                    { tab_button(DeviceKind::Camera, tab.clone()) }

                    <div class="device-settings-left-panel__quick-toggles" aria-label="Quick media toggles">
                        <button
                            type="button"
                            class={classes!(
                                "device-settings-left-panel__quick-btn",
                                if props.mic_enabled {
                                    Some("device-settings-left-panel__quick-btn--on")
                                } else {
                                    Some("device-settings-left-panel__quick-btn--off")
                                }
                            )}
                            onclick={props.on_toggle_mic.reform(|_| ())}
                            aria-label={if props.mic_enabled { "Turn microphone off" } else { "Turn microphone on" }}
                            title={if props.mic_enabled { "Microphone on" } else { "Microphone off" }}
                        >
                            <img
                                class="device-settings-left-panel__quick-icon"
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
                                "device-settings-left-panel__quick-btn",
                                if props.camera_enabled {
                                    Some("device-settings-left-panel__quick-btn--on")
                                } else {
                                    Some("device-settings-left-panel__quick-btn--off")
                                }
                            )}
                            onclick={props.on_toggle_camera.reform(|_| ())}
                            aria-label={if props.camera_enabled { "Turn camera off" } else { "Turn camera on" }}
                            title={if props.camera_enabled { "Camera on" } else { "Camera off" }}
                        >
                            <img
                                class="device-settings-left-panel__quick-icon"
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

                <section class="device-settings-left-panel__main">
                    <header class="device-settings-left-panel__header">
                        <h3 id="device-settings-title" class="device-settings-left-panel__section-title">{ (*tab).title() }</h3>
                        <button
                            type="button"
                            class="device-settings-left-panel__close"
                            aria-label="Close"
                            onclick={props.on_close.reform(|_| ())}
                        >
                            {"×"}
                        </button>
                    </header>

                    { current_list }

                    if matches!(*tab, DeviceKind::Input) {
                        <div class="device-settings-left-panel__slider-row">
                            { level_slider(props.input_level, on_input_level) }
                        </div>
                    }

                    if matches!(*tab, DeviceKind::Output) {
                        <div class="device-settings-left-panel__slider-row">
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
        "device-settings-left-panel__nav-btn",
        is_active.then_some("device-settings-left-panel__nav-btn--active")
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

fn render_device_list(devices: &[DeviceEntry], selected: UseStateHandle<&'static str>) -> Html {
    html! {
        <div class="device-settings-left-panel__list">
            {
                for devices.iter().map(|entry| {
                    let is_selected = *selected == entry.id;
                    let selected_class = is_selected.then_some("device-settings-left-panel__item--selected");
                    let onclick = {
                        let selected = selected.clone();
                        let id = entry.id;
                        Callback::from(move |_| selected.set(id))
                    };
                    html! {
                        <button
                            type="button"
                            class={classes!("device-settings-left-panel__item", selected_class)}
                            onclick={onclick}
                        >
                            <span>{ entry.label }</span>
                        </button>
                    }
                })
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
            class="device-settings-left-panel__slider"
            type="range"
            min={MIN_LEVEL.to_string()}
            max={MAX_LEVEL.to_string()}
            value={value.to_string()}
            style={format!("--pct: {:.2}%;", pct)}
            oninput={oninput}
        />
    }
}
