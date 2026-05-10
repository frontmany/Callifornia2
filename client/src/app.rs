//! Root shell: wires components and document-wide effects (e.g. theme class on `<html>`).

use std::collections::HashMap;

use crate::components::{JoinRoomEntry, MainMenu, NicknameEntry, Room};
use crate::connector_api::{SignalingReadyResponse, logout_best_effort, renew_session};
use crate::signaling::{
    RoomSnapshot, ServerErrorCode, SignalingClient, SignalingEvent, SignalingIntent, SignalingStart,
};
use crate::theme::Theme;
use crate::webrtc_room::WebRtcRoomSession;

use gloo_timers::callback::Interval;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue, UnwrapThrowExt};
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
    let screen = use_state(|| AppScreen::Nickname);
    let session_id = use_state(|| Option::<String>::None);
    let handoff_complete = use_state(|| false);
    // When true, session TTL is refreshed by signaling; stop HTTP renewal on the connector.
    let signaling_ws_connected = use_state(|| false);
    let signaling_start = use_state(|| Option::<SignalingStart>::None);
    let signaling_client = use_mut_ref(|| Option::<SignalingClient>::None);
    let active_room = use_state(|| Option::<ActiveRoom>::None);
    let app_error = use_state(|| Option::<String>::None);
    // WebRTC session (non-reactive; lifecycle follows active_room).
    let webrtc_session = use_mut_ref(|| Option::<WebRtcRoomSession>::None);
    // Remote streams keyed by MediaStream.id; updates trigger Room re-render.
    let remote_streams = use_state(|| HashMap::<String, JsValue>::new());

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
            let mut updated = (*settings_state).clone();
            updated.theme = next;
            settings_state.set(updated);
        })
    };

    let on_nickname_success = {
        let screen = screen.clone();
        let session_id = session_id.clone();
        let handoff_complete = handoff_complete.clone();
        let signaling_ws_connected = signaling_ws_connected.clone();
        let signaling_start = signaling_start.clone();
        let active_room = active_room.clone();
        let app_error = app_error.clone();
        Callback::from(move |new_session_id: String| {
            session_id.set(Some(new_session_id));
            handoff_complete.set(false);
            signaling_ws_connected.set(false);
            signaling_start.set(None);
            active_room.set(None);
            app_error.set(None);
            screen.set(AppScreen::MainMenu);
        })
    };

    let on_toggle_mic = {
        let settings_state = settings_state.clone();
        Callback::from(move |_| {
            let mut next = (*settings_state).clone();
            next.mic_enabled = !next.mic_enabled;
            settings_state.set(next);
        })
    };

    let on_toggle_camera = {
        let settings_state = settings_state.clone();
        Callback::from(move |_| {
            let mut next = (*settings_state).clone();
            next.camera_enabled = !next.camera_enabled;
            settings_state.set(next);
        })
    };

    let on_input_level_change = {
        let settings_state = settings_state.clone();
        Callback::from(move |next_level: u32| {
            let mut next = (*settings_state).clone();
            next.input_level = next_level;
            settings_state.set(next);
        })
    };

    let on_output_level_change = {
        let settings_state = settings_state.clone();
        Callback::from(move |next_level: u32| {
            let mut next = (*settings_state).clone();
            next.output_level = next_level;
            settings_state.set(next);
        })
    };

    let on_mic_device_change = {
        let settings_state = settings_state.clone();
        Callback::from(move |device_id: String| {
            let mut next = (*settings_state).clone();
            next.mic_device_id = device_id;
            settings_state.set(next);
        })
    };

    let on_speaker_device_change = {
        let settings_state = settings_state.clone();
        Callback::from(move |device_id: String| {
            let mut next = (*settings_state).clone();
            next.speaker_device_id = device_id;
            settings_state.set(next);
        })
    };

    let on_camera_device_change = {
        let settings_state = settings_state.clone();
        Callback::from(move |device_id: String| {
            let mut next = (*settings_state).clone();
            next.camera_device_id = device_id;
            settings_state.set(next);
        })
    };

    let on_back_to_nickname = {
        let screen = screen.clone();
        let session_id = session_id.clone();
        let signaling_ws_connected = signaling_ws_connected.clone();
        let signaling_start = signaling_start.clone();
        let active_room = active_room.clone();
        let handoff_complete = handoff_complete.clone();
        let app_error = app_error.clone();
        Callback::from(move |_| {
            session_id.set(None);
            signaling_ws_connected.set(false);
            signaling_start.set(None);
            active_room.set(None);
            handoff_complete.set(false);
            app_error.set(None);
            screen.set(AppScreen::Nickname);
        })
    };

    let on_open_join_room = {
        let screen = screen.clone();
        Callback::from(move |_| screen.set(AppScreen::JoinRoom))
    };

    let on_back_to_main_menu = {
        let screen = screen.clone();
        let session_id = session_id.clone();
        let signaling_ws_connected = signaling_ws_connected.clone();
        let signaling_start = signaling_start.clone();
        let active_room = active_room.clone();
        let signaling_client = signaling_client.clone();
        let app_error = app_error.clone();
        Callback::from(move |_| {
            if let Some(client) = signaling_client.borrow().as_ref() {
                if let (Some(session), Some(room)) = ((*session_id).clone(), (*active_room).clone())
                {
                    let _ = client.leave(&session, &room.room_id, &room.participants);
                }
            }
            signaling_start.set(None);
            active_room.set(None);
            signaling_ws_connected.set(false);
            app_error.set(None);
            screen.set(AppScreen::MainMenu);
        })
    };

    let on_signaling_ready = {
        let signaling_start = signaling_start.clone();
        let app_error = app_error.clone();
        Callback::from(
            move |(response, intent): (SignalingReadyResponse, SignalingIntent)| {
                app_error.set(None);
                signaling_start.set(Some(SignalingStart::new(response, intent)));
            },
        )
    };

    {
        let start = (*signaling_start).clone();
        let client_ref = signaling_client.clone();
        let screen = screen.clone();
        let active_room = active_room.clone();
        let signaling_start = signaling_start.clone();
        let signaling_ws_connected = signaling_ws_connected.clone();
        let handoff_complete = handoff_complete.clone();
        let app_error = app_error.clone();
        let webrtc_session = webrtc_session.clone();
        use_effect_with(start, move |start| {
            *client_ref.borrow_mut() = None;

            if let Some(start) = start.clone() {
                let events = {
                    let screen = screen.clone();
                    let active_room = active_room.clone();
                    let signaling_ws_connected = signaling_ws_connected.clone();
                    let handoff_complete = handoff_complete.clone();
                    let app_error = app_error.clone();
                    let webrtc_session = webrtc_session.clone();
                    Callback::from(move |event: SignalingEvent| match event {
                        SignalingEvent::Connected => {
                            signaling_ws_connected.set(true);
                            handoff_complete.set(true);
                        }
                        SignalingEvent::Disconnected { reason } => {
                            signaling_ws_connected.set(false);
                            if active_room.is_none() {
                                if let Some(reason) = reason {
                                    app_error.set(Some(format!(
                                        "Signaling disconnected before joining room: {reason}"
                                    )));
                                }
                                signaling_start.set(None);
                                screen.set(AppScreen::MainMenu);
                            }
                        }
                        SignalingEvent::RoomReady(snapshot) => {
                            active_room.set(Some(ActiveRoom::from(snapshot)));
                            app_error.set(None);
                            screen.set(AppScreen::Room);
                        }
                        SignalingEvent::ParticipantJoined(nickname) => {
                            active_room.set(active_room_with_participant(
                                (*active_room).clone(),
                                nickname,
                            ));
                        }
                        SignalingEvent::ParticipantLeft(nickname) => {
                            active_room.set(active_room_without_participant(
                                (*active_room).clone(),
                                &nickname,
                            ));
                        }
                        SignalingEvent::Left { room_id } => {
                            active_room.set(None);
                            signaling_start.set(None);
                            signaling_ws_connected.set(false);
                            app_error.set(Some(format!("Left room {room_id}.")));
                            screen.set(AppScreen::MainMenu);
                        }
                        SignalingEvent::RoomClosed { room_id, reason } => {
                            active_room.set(None);
                            signaling_start.set(None);
                            signaling_ws_connected.set(false);
                            app_error.set(Some(format!("Room {room_id} closed: {reason}")));
                            screen.set(AppScreen::MainMenu);
                        }
                        SignalingEvent::Error(error) => {
                            if error.code == Some(ServerErrorCode::InvalidRoomAssignment) {
                                active_room.set(None);
                                signaling_start.set(None);
                                signaling_ws_connected.set(false);
                                handoff_complete.set(false);
                                screen.set(AppScreen::MainMenu);
                            }
                            if error.code == Some(ServerErrorCode::SfuCapacityExhausted) {
                                active_room.set(None);
                                signaling_start.set(None);
                                signaling_ws_connected.set(false);
                                handoff_complete.set(false);
                                screen.set(AppScreen::MainMenu);
                            }
                            app_error.set(Some(error.message));
                        }
                        SignalingEvent::ServiceUnavailable {
                            dependency,
                            retry_after_ms,
                        } => {
                            active_room.set(None);
                            signaling_start.set(None);
                            signaling_ws_connected.set(false);
                            app_error.set(Some(format!(
                                "{dependency} unavailable. Retry after {retry_after_ms} ms."
                            )));
                            screen.set(AppScreen::MainMenu);
                        }
                        // Route WebRTC media events to the active session.
                        SignalingEvent::RemoteSdp { sdp, sdp_type } => {
                            if let Some(session) = webrtc_session.borrow().as_ref() {
                                session.handle_remote_sdp(sdp, sdp_type);
                            }
                        }
                        SignalingEvent::RemoteIceCandidate { candidate, sdp_mid } => {
                            if let Some(session) = webrtc_session.borrow().as_ref() {
                                session.handle_remote_ice_candidate(candidate, sdp_mid);
                            }
                        }
                    })
                };

                match SignalingClient::connect(start, events) {
                    Ok(client) => {
                        *client_ref.borrow_mut() = Some(client);
                    }
                    Err(err) => {
                        app_error.set(Some(err));
                        screen.set(AppScreen::MainMenu);
                    }
                }
            }

            move || {
                *client_ref.borrow_mut() = None;
            }
        });
    }

    {
        let session_id_value = (*session_id).clone();
        let ws_connected = *signaling_ws_connected;
        use_effect_with(
            (session_id_value, ws_connected),
            move |(session_id, ws_connected)| {
                let interval_handle = if let Some(session) = session_id.clone() {
                    if *ws_connected {
                        None
                    } else {
                        let s0 = session.clone();
                        spawn_local(async move {
                            let _ = renew_session(&s0).await;
                        });
                        let s_tick = session;
                        Some(Interval::new(
                            CONNECTOR_SESSION_RENEW_INTERVAL_MS,
                            move || {
                                let s = s_tick.clone();
                                spawn_local(async move {
                                    let _ = renew_session(&s).await;
                                });
                            },
                        ))
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

    // ── WebRTC session lifecycle ──────────────────────────────────────────────
    // Keyed by room_id (not the full active_room) so participant join/leave
    // events do not tear down and restart the peer connection.
    {
        let webrtc_room_dep = (*active_room).as_ref().map(|r| r.room_id.clone());
        let signaling_client = signaling_client.clone();
        let settings_snapshot = (*settings_state).clone();
        let webrtc_session = webrtc_session.clone();
        let remote_streams = remote_streams.clone();
        use_effect_with(webrtc_room_dep, move |room_id_opt| {
            if room_id_opt.is_some() {
                let transport = signaling_client.borrow().as_ref().map(|c| c.transport());
                if let Some(transport) = transport {
                    let remote_streams_cb = remote_streams.clone();
                    let on_streams =
                        Callback::from(move |streams: HashMap<String, JsValue>| {
                            remote_streams_cb.set(streams);
                        });
                    match WebRtcRoomSession::start(
                        transport,
                        settings_snapshot.mic_enabled,
                        settings_snapshot.camera_enabled,
                        settings_snapshot.mic_device_id.clone(),
                        settings_snapshot.camera_device_id.clone(),
                        on_streams,
                    ) {
                        Ok(session) => *webrtc_session.borrow_mut() = Some(session),
                        Err(_) => {}
                    }
                }
            }
            let webrtc_session = webrtc_session.clone();
            let remote_streams = remote_streams.clone();
            move || {
                if let Some(session) = webrtc_session.borrow_mut().take() {
                    session.close();
                }
                remote_streams.set(HashMap::new());
            }
        });
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
                    settings_state={(*settings_state).clone()}
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
                    on_signaling_ready={on_signaling_ready.clone()}
                />
            },
            AppScreen::JoinRoom => html! {
                <JoinRoomEntry
                    on_back={on_back_to_main_menu}
                    settings_state={(*settings_state).clone()}
                    on_theme_change={on_theme}
                    on_toggle_mic={on_toggle_mic}
                    on_toggle_camera={on_toggle_camera}
                    on_input_level_change={on_input_level_change}
                    on_output_level_change={on_output_level_change}
                    on_mic_device_change={on_mic_device_change}
                    on_speaker_device_change={on_speaker_device_change}
                    on_camera_device_change={on_camera_device_change}
                    session_id={(*session_id).clone().unwrap_or_default()}
                    on_signaling_ready={on_signaling_ready.clone()}
                />
            },
            AppScreen::Room => {
                if let Some(room) = (*active_room).clone() {
                    // Build participant_media in participant list order.
                    // Local stream goes to the slot for your_nickname; remote streams
                    // are assigned to other participants by arrival order (i-th remote
                    // stream → i-th non-self participant). This works exactly for
                    // 2-person calls and approximately for larger rooms until the SFU
                    // encodes per-sender stream IDs in signaling metadata.
                    let streams_snapshot = (*remote_streams).clone();
                    let local_js: Option<JsValue> =
                        webrtc_session.borrow().as_ref().and_then(|s| s.local_stream());
                    let remote_stream_list: Vec<JsValue> =
                        streams_snapshot.values().cloned().collect();
                    let other_nicks: Vec<&String> = room
                        .participants
                        .iter()
                        .filter(|n| *n != &room.your_nickname)
                        .collect();
                    let participant_media: Vec<Option<JsValue>> = room
                        .participants
                        .iter()
                        .map(|nick| {
                            if nick == &room.your_nickname {
                                local_js.clone()
                            } else {
                                let idx = other_nicks
                                    .iter()
                                    .position(|n| *n == nick)
                                    .unwrap_or(0);
                                remote_stream_list.get(idx).cloned()
                            }
                        })
                        .collect();

                    html! {
                        <Room
                            room_id={room.room_id}
                            your_nickname={room.your_nickname}
                            participants={room.participants}
                            participant_media={participant_media}
                            settings_state={(*settings_state).clone()}
                            on_theme_change={on_theme.clone()}
                            on_toggle_mic={on_toggle_mic.clone()}
                            on_toggle_camera={on_toggle_camera.clone()}
                            on_input_level_change={on_input_level_change.clone()}
                            on_output_level_change={on_output_level_change.clone()}
                            on_mic_device_change={on_mic_device_change.clone()}
                            on_speaker_device_change={on_speaker_device_change.clone()}
                            on_camera_device_change={on_camera_device_change.clone()}
                            on_end_call={on_back_to_main_menu.clone()}
                        />
                    }
                } else {
                    html! {
                        <main class="font-manrope text-on-background">
                            <p>{ (*app_error).clone().unwrap_or_else(|| "Connecting to room...".to_owned()) }</p>
                        </main>
                    }
                }
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveRoom {
    room_id: String,
    your_nickname: String,
    participants: Vec<String>,
}

impl From<RoomSnapshot> for ActiveRoom {
    fn from(snapshot: RoomSnapshot) -> Self {
        Self {
            room_id: snapshot.room_id,
            your_nickname: snapshot.your_nickname,
            participants: unique_participants(snapshot.participants),
        }
    }
}

fn active_room_with_participant(room: Option<ActiveRoom>, nickname: String) -> Option<ActiveRoom> {
    room.map(|mut room| {
        if !room
            .participants
            .iter()
            .any(|existing| existing == &nickname)
        {
            room.participants.push(nickname);
        }
        room
    })
}

fn active_room_without_participant(room: Option<ActiveRoom>, nickname: &str) -> Option<ActiveRoom> {
    room.map(|mut room| {
        room.participants.retain(|existing| existing != nickname);
        room
    })
}

fn unique_participants(participants: Vec<String>) -> Vec<String> {
    participants
        .into_iter()
        .fold(Vec::new(), |mut unique, name| {
            if !unique.iter().any(|existing| existing == &name) {
                unique.push(name);
            }
            unique
        })
}

#[derive(Clone, PartialEq, Eq)]
pub struct SettingsState {
    pub theme: Theme,
    pub mic_enabled: bool,
    pub camera_enabled: bool,
    pub input_level: u32,
    pub output_level: u32,
    pub mic_device_id: String,
    pub speaker_device_id: String,
    pub camera_device_id: String,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            theme: Theme::Light,
            mic_enabled: true,
            camera_enabled: false,
            input_level: 100,
            output_level: 100,
            mic_device_id: "default".to_owned(),
            speaker_device_id: "default".to_owned(),
            camera_device_id: "default".to_owned(),
        }
    }
}

fn initial_theme_from_system() -> Theme {
    let Some(window) = web_sys::window() else {
        return Theme::Light;
    };

    match window.match_media("(prefers-color-scheme: dark)") {
        Ok(Some(media_query)) if media_query.matches() => Theme::Dark,
        _ => Theme::Light,
    }
}
