use crate::app::SettingsState;
use crate::components::SettingsPanel;
use crate::connector_api::{self, SignalingReadyResponse};
use crate::signaling::SignalingIntent;
use crate::theme::Theme;
use wasm_bindgen_futures::spawn_local;
use web_sys::{HtmlFormElement, HtmlInputElement, KeyboardEvent};
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct JoinRoomEntryProps {
    pub on_back: Callback<()>,
    pub settings_state: SettingsState,
    pub on_theme_change: Callback<Theme>,
    pub on_toggle_mic: Callback<()>,
    pub on_toggle_camera: Callback<()>,
    pub on_input_level_change: Callback<u32>,
    pub on_output_level_change: Callback<u32>,
    pub on_mic_device_change: Callback<String>,
    pub on_speaker_device_change: Callback<String>,
    pub on_camera_device_change: Callback<String>,
    pub session_id: String,
    pub on_signaling_ready: Callback<(SignalingReadyResponse, SignalingIntent)>,
}

#[function_component]
pub fn JoinRoomEntry(props: &JoinRoomEntryProps) -> Html {
    let is_settings_open = use_state(|| false);
    let room_target = use_state(String::new);
    let validation_error = use_state(|| Option::<String>::None);
    let action_busy = use_state(|| false);
    let form_ref = use_node_ref();

    let open_settings = {
        let is_settings_open = is_settings_open.clone();
        Callback::from(move |_| is_settings_open.set(true))
    };

    let close_settings = {
        let is_settings_open = is_settings_open.clone();
        Callback::from(move |_| is_settings_open.set(false))
    };

    let on_input = {
        let room_target = room_target.clone();
        let validation_error = validation_error.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlInputElement = event.target_unchecked_into();
            let value = input.value();
            let is_empty = value.trim().is_empty();
            room_target.set(value);
            if is_empty {
                validation_error.set(Some("Enter a room ID.".to_owned()));
            } else {
                validation_error.set(None);
            }
        })
    };

    let run_submit = {
        let room_target = room_target.clone();
        let validation_error = validation_error.clone();
        let action_busy = action_busy.clone();
        let session_id = props.session_id.clone();
        let on_signaling_ready = props.on_signaling_ready.clone();
        Callback::from(move |_| {
            if *action_busy {
                return;
            }

            let value = (*room_target).trim().to_owned();
            if value.is_empty() {
                validation_error.set(Some("Enter a room ID.".to_owned()));
                return;
            }
            if !is_uuid_like(&value) {
                validation_error.set(Some("Room ID must be a UUID.".to_owned()));
                return;
            }

            validation_error.set(None);
            action_busy.set(true);
            let validation_error = validation_error.clone();
            let action_busy = action_busy.clone();
            let session_id = session_id.clone();
            let on_signaling_ready = on_signaling_ready.clone();
            spawn_local(async move {
                match connector_api::join(&session_id, &value).await {
                    Ok(response) => {
                        on_signaling_ready
                            .emit((response, SignalingIntent::Join { room_id: value }));
                    }
                    Err(err) => validation_error.set(Some(err)),
                }
                action_busy.set(false);
            });
        })
    };

    let on_submit = {
        let run_submit = run_submit.clone();
        Callback::from(move |event: SubmitEvent| {
            event.prevent_default();
            run_submit.emit(());
        })
    };

    let on_input_keydown = {
        let form_ref = form_ref.clone();
        Callback::from(move |event: KeyboardEvent| {
            let key = event.key();
            if key != "Enter" && key != "NumpadEnter" {
                return;
            }
            event.prevent_default();
            if let Some(form) = form_ref.cast::<HtmlFormElement>() {
                let _ = form.request_submit();
            }
        })
    };

    let has_error = validation_error.is_some();
    let mut input_classes = classes!(
        "join-room-entry__input",
        "font-manrope",
        "body-lg",
        "text-on-surface"
    );
    if has_error {
        input_classes.push("join-room-entry__input--error");
    }

    html! {
        <div class="join-room-entry-page">
            <div class="join-room-entry__actions join-room-entry__actions--floating">
                <button
                    type="button"
                    class="join-room-entry__icon-btn join-room-entry__icon-btn--settings"
                    aria-label="Settings"
                    aria-haspopup="dialog"
                    onclick={open_settings}
                >
                    <svg
                        xmlns="http://www.w3.org/2000/svg"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.9"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        aria-hidden="true"
                    >
                        <circle cx="12" cy="12" r="3" />
                        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33h.01a1.65 1.65 0 0 0 .99-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 .99 1.51h.01a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.01a1.65 1.65 0 0 0 1.51.99H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51.99z" />
                    </svg>
                </button>
            </div>

            <main class="join-room-entry font-manrope text-on-background">
                <section class="join-room-entry__card" aria-label="Join room card">
                    <div class="join-room-entry__avatar" aria-hidden="true">
                        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="34" height="34" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M10 13a5 5 0 0 1 0-7l1-1a5 5 0 0 1 7 7l-1 1" />
                            <path d="M14 11a5 5 0 0 1 0 7l-1 1a5 5 0 0 1-7-7l1-1" />
                        </svg>
                    </div>

                    <h1 class="join-room-entry__title h1">{ "Join a room" }</h1>

                    <form ref={form_ref} class="join-room-entry__form" onsubmit={on_submit}>
                        <input
                            id="join-room-input"
                            class={input_classes}
                            type="text"
                            value={(*room_target).clone()}
                            oninput={on_input}
                            onkeydown={on_input_keydown}
                            placeholder="e.g. 2f7d1d3e-8e2a-4f0b-a7f2-90d34d77a3b5"
                            autocomplete="off"
                        />
                        <p class="join-room-entry__hint">
                            { "Paste the room UUID from your host." }
                        </p>

                        if let Some(message) = &*validation_error {
                            <p class="join-room-entry__error body-md" role="alert">{ message }</p>
                        }

                        <button class="join-room-entry__button" type="submit" disabled={*action_busy}>
                            <span class="join-room-entry__button-label">
                                { if *action_busy { "Joining..." } else { "Join room" } }
                            </span>
                            <span class="join-room-entry__button-arrow" aria-hidden="true">
                                <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                                    <path d="M5 12h14" />
                                    <path d="M13 6l6 6-6 6" />
                                </svg>
                            </span>
                        </button>

                        <button
                            type="button"
                            class="join-room-entry__back"
                            onclick={props.on_back.reform(|_| ())}
                        >
                            { "Back" }
                        </button>
                    </form>
                </section>
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
                    selected_mic_device_id={props.settings_state.mic_device_id.clone()}
                    selected_speaker_device_id={props.settings_state.speaker_device_id.clone()}
                    selected_camera_device_id={props.settings_state.camera_device_id.clone()}
                    on_mic_device_change={props.on_mic_device_change.clone()}
                    on_speaker_device_change={props.on_speaker_device_change.clone()}
                    on_camera_device_change={props.on_camera_device_change.clone()}
                />
            }
        </div>
    }
}

fn is_uuid_like(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23].into_iter().all(|idx| bytes[idx] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(idx, byte)| [8, 13, 18, 23].contains(&idx) || byte.is_ascii_hexdigit())
}
