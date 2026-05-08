use crate::components::SettingsPanel;
use crate::app::SettingsState;
use crate::theme::Theme;
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
    pub on_mic_device_change: Callback<&'static str>,
    pub on_speaker_device_change: Callback<&'static str>,
    pub on_camera_device_change: Callback<&'static str>,
}

#[function_component]
pub fn JoinRoomEntry(props: &JoinRoomEntryProps) -> Html {
    let is_settings_open = use_state(|| false);
    let room_target = use_state(String::new);
    let validation_error = use_state(|| Option::<String>::None);
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
        Callback::from(move |_| {
            let value = (*room_target).trim().to_owned();
            if value.is_empty() {
                validation_error.set(Some("Enter a room ID.".to_owned()));
                return;
            }

            validation_error.set(None);
            // TODO: call join-room API with `value` when backend flow is finalized.
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
        <div class="join-room-entry font-manrope text-on-background">
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

            <main class="join-room-entry__content">
            <section class="join-room-entry__card" aria-label="Join room card">
                <div class="join-room-entry__avatar" aria-hidden="true">
                    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="34" height="34" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M10 13a5 5 0 0 1 0-7l1-1a5 5 0 0 1 7 7l-1 1" />
                        <path d="M14 11a5 5 0 0 1 0 7l-1 1a5 5 0 0 1-7-7l1-1" />
                    </svg>
                </div>

                <h1 class="join-room-entry__title h1">{ "Join Room" }</h1>
                <p class="join-room-entry__subtitle body-md">{ "Enter your room ID to connect in seconds." }</p>

                <form ref={form_ref} class="join-room-entry__form" onsubmit={on_submit}>
                    <input
                        id="join-room-input"
                        class={input_classes}
                        type="text"
                        value={(*room_target).clone()}
                        oninput={on_input}
                        onkeydown={on_input_keydown}
                        placeholder="e.g. 33fa5b65"
                        autocomplete="off"
                    />
                    <p class="join-room-entry__hint body-md">{ "Room ID only. Use the code shared by the room host" }</p>

                    if let Some(message) = &*validation_error {
                        <p class="join-room-entry__error body-md" role="alert">{ message }</p>
                    }

                    <button class="join-room-entry__button" type="submit">
                        { "Join Room" }
                    </button>

                    <button
                        type="button"
                        class="join-room-entry__back body-md"
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
