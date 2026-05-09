//! Root shell: wires components and document-wide effects (e.g. theme class on `<html>`).

use crate::components::{JoinRoomEntry, MainMenu, NicknameEntry, Room};
use crate::connector_api::{logout_best_effort, renew_session};
use crate::theme::Theme;
use gloo_timers::callback::Interval;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, UnwrapThrowExt};
use wasm_bindgen_futures::spawn_local;
use web_sys::{Event, HtmlElement};
use yew::prelude::*;

/// Must stay below connector `SESSION_TTL_SEC` and match signaling `SESSION_RENEW_SEC` order of magnitude.
const CONNECTOR_SESSION_RENEW_INTERVAL_MS: u32 = 60_000;

#[function_component]
pub fn App() -> Html {
    let settings_state = use_state(|| SettingsState {
        theme: initial_theme_from_system(),
        ..SettingsState::default()
    });
    // DEV_ROOM_PREVIEW: set to `false` to restore normal flow (nickname → menu → join).
    const DEV_FORCE_ROOM_VIEW: bool = true;
    let screen = use_state(|| {
        if DEV_FORCE_ROOM_VIEW {
            AppScreen::Room
        } else {
            AppScreen::Nickname
        }
    });
    let session_id = use_state(|| Option::<String>::None);
    let handoff_complete = use_state(|| false);
    // When true, session TTL is refreshed by signaling; stop HTTP renewal on the connector.
    let signaling_ws_connected = use_state(|| false);

    {
        let theme = settings_state.theme;
        use_effect_with(theme, move |t| {
            let window = web_sys::window().unwrap_throw();
            let document = window.document().unwrap_throw();
            if let Some(root) = document.document_element() {
                let html: HtmlElement = root.dyn_into().unwrap_throw();
                html.set_class_name(t.html_class());
            }
        });
    }

    let on_theme = {
        let settings_state = settings_state.clone();
        Callback::from(move |next: Theme| {
            let mut updated = *settings_state;
            updated.theme = next;
            settings_state.set(updated);
        })
    };

    let on_nickname_success = {
        let screen = screen.clone();
        let session_id = session_id.clone();
        let handoff_complete = handoff_complete.clone();
        let signaling_ws_connected = signaling_ws_connected.clone();
        Callback::from(move |new_session_id: String| {
            session_id.set(Some(new_session_id));
            handoff_complete.set(false);
            signaling_ws_connected.set(false);
            screen.set(AppScreen::MainMenu);
        })
    };

    let on_toggle_mic = {
        let settings_state = settings_state.clone();
        Callback::from(move |_| {
            let mut next = *settings_state;
            next.mic_enabled = !next.mic_enabled;
            settings_state.set(next);
        })
    };

    let on_toggle_camera = {
        let settings_state = settings_state.clone();
        Callback::from(move |_| {
            let mut next = *settings_state;
            next.camera_enabled = !next.camera_enabled;
            settings_state.set(next);
        })
    };

    let on_input_level_change = {
        let settings_state = settings_state.clone();
        Callback::from(move |next_level: u32| {
            let mut next = *settings_state;
            next.input_level = next_level;
            settings_state.set(next);
        })
    };

    let on_output_level_change = {
        let settings_state = settings_state.clone();
        Callback::from(move |next_level: u32| {
            let mut next = *settings_state;
            next.output_level = next_level;
            settings_state.set(next);
        })
    };

    let on_mic_device_change = {
        let settings_state = settings_state.clone();
        Callback::from(move |device_id: &'static str| {
            let mut next = *settings_state;
            next.mic_device_id = device_id;
            settings_state.set(next);
        })
    };

    let on_speaker_device_change = {
        let settings_state = settings_state.clone();
        Callback::from(move |device_id: &'static str| {
            let mut next = *settings_state;
            next.speaker_device_id = device_id;
            settings_state.set(next);
        })
    };

    let on_camera_device_change = {
        let settings_state = settings_state.clone();
        Callback::from(move |device_id: &'static str| {
            let mut next = *settings_state;
            next.camera_device_id = device_id;
            settings_state.set(next);
        })
    };

    let on_back_to_nickname = {
        let screen = screen.clone();
        let session_id = session_id.clone();
        let signaling_ws_connected = signaling_ws_connected.clone();
        Callback::from(move |_| {
            session_id.set(None);
            signaling_ws_connected.set(false);
            screen.set(AppScreen::Nickname);
        })
    };

    let on_signaling_ws_connected = {
        let signaling_ws_connected = signaling_ws_connected.clone();
        Callback::from(move |connected: bool| {
            signaling_ws_connected.set(connected);
        })
    };

    let on_open_join_room = {
        let screen = screen.clone();
        Callback::from(move |_| screen.set(AppScreen::JoinRoom))
    };

    let on_back_to_main_menu = {
        let screen = screen.clone();
        let signaling_ws_connected = signaling_ws_connected.clone();
        Callback::from(move |_| {
            // Left in-call / room flow; treat as no signaling until WS opens again.
            signaling_ws_connected.set(false);
            screen.set(AppScreen::MainMenu);
        })
    };

    let on_handoff_complete = {
        let handoff_complete = handoff_complete.clone();
        // TODO: after create/join handoff, start a signaling WebSocket client and
        // process incoming ServerMessage events (joined, participant updates,
        // SDP/ICE, errors, transfer) to drive in-call UI state.
        Callback::from(move |_| handoff_complete.set(true))
    };

    {
        let session_id_value = (*session_id).clone();
        let ws_connected = *signaling_ws_connected;
        use_effect_with(
            (session_id_value, ws_connected),
            move |(session_id, ws_connected)| {
                let interval_handle =
                    if let Some(session) = session_id.clone() {
                        if *ws_connected {
                            None
                        } else {
                            let s0 = session.clone();
                            spawn_local(async move {
                                let _ = renew_session(&s0).await;
                            });
                            let s_tick = session;
                            Some(Interval::new(CONNECTOR_SESSION_RENEW_INTERVAL_MS, move || {
                                let s = s_tick.clone();
                                spawn_local(async move {
                                    let _ = renew_session(&s).await;
                                });
                            }))
                        }
                    } else {
                        None
                    };
                move || drop(interval_handle)
            },
        );
    }

    {
        let session_id_value = (*session_id).clone();
        let handoff_done = *handoff_complete;
        use_effect_with(
            (session_id_value, handoff_done),
            move |(session_id, handoff_done)| {
                let listener = if let Some(session_id) = session_id.clone() {
                    if !*handoff_done {
                        let listener_session_id = session_id.clone();
                        let callback = Closure::wrap(Box::new(move |_event: Event| {
                            logout_best_effort(listener_session_id.clone());
                        })
                            as Box<dyn FnMut(_)>);
                        let window = web_sys::window().unwrap_throw();
                        window
                            .add_event_listener_with_callback(
                                "pagehide",
                                callback.as_ref().unchecked_ref(),
                            )
                            .unwrap_throw();
                        Some((window, callback))
                    } else {
                        None
                    }
                } else {
                    None
                };
                move || {
                    if let Some((window, callback)) = listener {
                        let _ = window.remove_event_listener_with_callback(
                            "pagehide",
                            callback.as_ref().unchecked_ref(),
                        );
                        drop(callback);
                    }
                }
            },
        );
    }

    html! {
        match *screen {
            AppScreen::Nickname => html! {
                <>
                    <NicknameEntry on_success={on_nickname_success} />
                </>
            },
            AppScreen::MainMenu => html! {
                <MainMenu
                    settings_state={*settings_state}
                    on_theme_change={on_theme.clone()}
                    on_toggle_mic={on_toggle_mic.clone()}
                    on_toggle_camera={on_toggle_camera.clone()}
                    on_input_level_change={on_input_level_change.clone()}
                    on_output_level_change={on_output_level_change.clone()}
                    on_mic_device_change={on_mic_device_change.clone()}
                    on_speaker_device_change={on_speaker_device_change.clone()}
                    on_camera_device_change={on_camera_device_change.clone()}
                    on_back={on_back_to_nickname}
                    on_join_room={on_open_join_room}
                    session_id={(*session_id).clone().unwrap_or_default()}
                    on_handoff_complete={on_handoff_complete}
                />
            },
            AppScreen::JoinRoom => html! {
                <JoinRoomEntry
                    on_back={on_back_to_main_menu}
                    settings_state={*settings_state}
                    on_theme_change={on_theme}
                    on_toggle_mic={on_toggle_mic}
                    on_toggle_camera={on_toggle_camera}
                    on_input_level_change={on_input_level_change}
                    on_output_level_change={on_output_level_change}
                    on_mic_device_change={on_mic_device_change}
                    on_speaker_device_change={on_speaker_device_change}
                    on_camera_device_change={on_camera_device_change}
                />
            },
            AppScreen::Room => html! {
                <Room
                    settings_state={*settings_state}
                    on_theme_change={on_theme.clone()}
                    on_toggle_mic={on_toggle_mic.clone()}
                    on_toggle_camera={on_toggle_camera.clone()}
                    on_input_level_change={on_input_level_change.clone()}
                    on_output_level_change={on_output_level_change.clone()}
                    on_mic_device_change={on_mic_device_change.clone()}
                    on_speaker_device_change={on_speaker_device_change.clone()}
                    on_camera_device_change={on_camera_device_change.clone()}
                    on_end_call={on_back_to_main_menu.clone()}
                    on_signaling_connected={Some(on_signaling_ws_connected.clone())}
                />
            },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppScreen {
    Nickname,
    MainMenu,
    JoinRoom,
    Room,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SettingsState {
    pub theme: Theme,
    pub mic_enabled: bool,
    pub camera_enabled: bool,
    pub input_level: u32,
    pub output_level: u32,
    pub mic_device_id: &'static str,
    pub speaker_device_id: &'static str,
    pub camera_device_id: &'static str,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            theme: Theme::Light,
            mic_enabled: true,
            camera_enabled: false,
            input_level: 100,
            output_level: 100,
            mic_device_id: "mic-default",
            speaker_device_id: "spk-default",
            camera_device_id: "cam-integrated",
        }
    }
}

fn initial_theme_from_system() -> Theme {
    // TODO: keep theme fully system-driven; do not persist or restore user override.
    let Some(window) = web_sys::window() else {
        return Theme::Light;
    };

    match window.match_media("(prefers-color-scheme: dark)") {
        Ok(Some(media_query)) if media_query.matches() => Theme::Dark,
        _ => Theme::Light,
    }
}
