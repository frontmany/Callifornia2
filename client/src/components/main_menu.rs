//! Main screen after nickname: room choice and global chrome.

#[path = "main_menu/icons.rs"]
mod icons;

use crate::components::SettingsPanel;
use crate::connector_api;
use crate::app::SettingsState;
use crate::theme::Theme;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct MainMenuProps {
    pub settings_state: SettingsState,
    pub on_theme_change: Callback<Theme>,
    pub on_toggle_mic: Callback<()>,
    pub on_toggle_camera: Callback<()>,
    pub on_input_level_change: Callback<u32>,
    pub on_output_level_change: Callback<u32>,
    pub on_mic_device_change: Callback<&'static str>,
    pub on_speaker_device_change: Callback<&'static str>,
    pub on_camera_device_change: Callback<&'static str>,
    pub on_back: Callback<()>,
    pub on_join_room: Callback<()>,
    pub session_id: String,
    pub on_handoff_complete: Callback<()>,
}

#[function_component]
pub fn MainMenu(props: &MainMenuProps) -> Html {
    let is_settings_open = use_state(|| false);
    let action_error = use_state(|| Option::<String>::None);
    let action_busy = use_state(|| false);

    let open_settings = {
        let is_settings_open = is_settings_open.clone();
        Callback::from(move |_| is_settings_open.set(true))
    };

    let close_settings = {
        let is_settings_open = is_settings_open.clone();
        Callback::from(move |_| is_settings_open.set(false))
    };

    let on_create = {
        let session_id = props.session_id.clone();
        let action_error = action_error.clone();
        let action_busy = action_busy.clone();
        let on_handoff_complete = props.on_handoff_complete.clone();
        Callback::from(move |_| {
            if *action_busy {
                return;
            }
            action_busy.set(true);
            action_error.set(None);
            let session_id = session_id.clone();
            let action_error = action_error.clone();
            let action_busy = action_busy.clone();
            let on_handoff_complete = on_handoff_complete.clone();
            spawn_local(async move {
                match connector_api::create(&session_id).await {
                    Ok(_response) => on_handoff_complete.emit(()),
                    Err(err) => action_error.set(Some(err)),
                }
                action_busy.set(false);
            });
        })
    };

    let on_join = props.on_join_room.reform(|_| ());

    html! {
        <div class="main-menu font-manrope text-on-background">
            <div class="main-menu__actions main-menu__actions--floating">
                <button
                    type="button"
                    class="main-menu__icon-btn main-menu__icon-btn--settings"
                    aria-label="Settings"
                    aria-haspopup="dialog"
                    onclick={open_settings}
                >
                    { icons::settings_gear_icon() }
                </button>
            </div>

            <main class="main-menu__content">
                <header class="main-menu__heading">
                    <h1 class="h1 main-menu__title">{ "Get connected" }</h1>
                    <p class="body-lg main-menu__subtitle">{ "Start a new space or drop into an existing one." }</p>
                </header>

                <section class="main-menu__cards">
                    <button type="button" class="main-menu__card" onclick={on_create} disabled={*action_busy}>
                        <span class="main-menu__card-icon main-menu__card-icon--primary" aria-hidden="true">
                            { icons::create_plus_icon() }
                        </span>
                        <span class="main-menu__card-title h2">{ "Create Room" }</span>
                        <span class="main-menu__card-text body-md">{ "Start a fresh session and invite your team." }</span>
                    </button>

                    <button type="button" class="main-menu__card" onclick={on_join} disabled={*action_busy}>
                        <span class="main-menu__card-icon main-menu__card-icon--secondary" aria-hidden="true">
                            { icons::join_enter_icon() }
                        </span>
                        <span class="main-menu__card-title h2">{ "Join Room" }</span>
                        <span class="main-menu__card-text body-md">{ "Enter a link or code to hop right in." }</span>
                    </button>
                </section>

                if let Some(err) = &*action_error {
                    <p class="body-md" role="alert">{ err }</p>
                }

                <button
                    type="button"
                    class="main-menu__back body-md"
                    onclick={props.on_back.reform(|_| ())}
                >
                    <span class="main-menu__back-arrow" aria-hidden="true">
                        { icons::back_arrow_icon() }
                    </span>
                    <span>{ "Go Back" }</span>
                </button>
            </main>

            if *is_settings_open {
                <SettingsPanel
                    on_close={close_settings}
                    theme={props.settings_state.theme}
                    on_theme_change={props.on_theme_change.clone()}
                    mic_enabled={props.settings_state.mic_enabled}
                    camera_enabled={props.settings_state.camera_enabled}
                    input_level={props.settings_state.input_level}
                    output_level={props.settings_state.output_level}
                    on_toggle_mic={props.on_toggle_mic.clone()}
                    on_toggle_camera={props.on_toggle_camera.clone()}
                    on_input_level_change={props.on_input_level_change.clone()}
                    on_output_level_change={props.on_output_level_change.clone()}
                    selected_mic_device_id={props.settings_state.mic_device_id}
                    selected_speaker_device_id={props.settings_state.speaker_device_id}
                    selected_camera_device_id={props.settings_state.camera_device_id}
                    on_mic_device_change={props.on_mic_device_change.clone()}
                    on_speaker_device_change={props.on_speaker_device_change.clone()}
                    on_camera_device_change={props.on_camera_device_change.clone()}
                />
            }
        </div>
    }
}
