//! WebRTC session for an in-call room: peer connection, local media capture, and remote stream routing.
//!
//! # Flow
//! 1. `WebRtcRoomSession::start()` creates an `RTCPeerConnection` with a public STUN server and
//!    sets up `onicecandidate` / `ontrack` callbacks.
//! 2. `getUserMedia` is called asynchronously; any local tracks are added to the PC.
//! 3. `createOffer` → `setLocalDescription` → SDP sent via `SignalingTransport`.
//! 4. When signaling delivers a remote SDP answer, `handle_remote_sdp` applies it.
//! 5. ICE candidates are exchanged in both directions via `handle_remote_ice_candidate` /
//!    `send_ice_candidate`.
//! 6. `ontrack` events merge remote tracks into one [`MediaStream`] per **publisher
//!    nickname**. The SFU names relay tracks `publisherId|sourceTrackId` (see
//!    `Room::buildRelayTrackId` in the native SFU); the browser exposes that id on
//!    [`MediaStreamTrack::id`], so we split on `|` and use the prefix as the map key.
//!    The `on_streams` callback receives `HashMap<nickname, MediaStream as JsValue>`.
//! 7. `close()` stops local tracks and closes the PC.
//!
//! ## Mid-call device / mute changes
//!
//! [`WebRtcRoomSession::apply_local_media`] runs a fresh `getUserMedia` with the same
//! [`media_track_constraint`] rules as the initial capture, then updates the peer connection
//! via [`RTCRtpSender::replace_track`](https://developer.mozilla.org/en-US/docs/Web/API/RTCRtpSender/replaceTrack)
//! (implemented through `replaceTrack` on the stored sender objects). Adding a track where
//! none existed uses `addTrack` and is followed by a **new offer** to the SFU, because a new
//! outgoing transceiver requires renegotiation.
//!
//! ## Input / output level
//! - **Input (mic)** — после `getUserMedia` аудио проходит через [`GainNode`] (Web Audio);
//!   громкость задаётся как `input_level / 100`. Слайдер без повторного `getUserMedia`
//!   обновляет [`AudioParam::value`] через [`WebRtcRoomSession::set_input_level`].
//! - **Output (динамики)** — глобальный коэффициент [`HtmlMediaElement::volume`] на все `<video>`
//!   в области звонка (см. `apply_global_output_volume` в `components/room.rs`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use js_sys::{Array, Object, Reflect};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    MediaStream, MediaStreamConstraints, MediaStreamTrack, RtcPeerConnection,
    RtcPeerConnectionIceEvent, RtcSdpType, RtcSessionDescriptionInit, RtcTrackEvent,
};
use yew::Callback;

use crate::signaling::SignalingTransport;

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteParticipantMedia {
    pub camera: Option<JsValue>,
    pub screen: Option<JsValue>,
}

impl RemoteParticipantMedia {
    fn empty() -> Self {
        Self {
            camera: None,
            screen: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebRtcMediaSeverity {
    Warning,
    Recoverable,
    Fatal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebRtcMediaError {
    pub severity: WebRtcMediaSeverity,
    pub stage: &'static str,
    pub message: String,
    pub technical: String,
}

pub struct WebRtcRoomSession {
    pc: RtcPeerConnection,
    signaling: SignalingTransport,
    local_stream: Rc<RefCell<Option<MediaStream>>>,
    screen_stream: Rc<RefCell<Option<MediaStream>>>,
    audio_sender: Rc<RefCell<Option<JsValue>>>,
    video_sender: Rc<RefCell<Option<JsValue>>>,
    screen_sender: Rc<RefCell<Option<JsValue>>>,
    /// `AudioContext` for mic gain (if any).
    mic_audio_ctx: Rc<RefCell<Option<JsValue>>>,
    /// `GainNode.gain` [`AudioParam`](https://developer.mozilla.org/en-US/docs/Web/API/AudioParam) for live slider updates.
    mic_gain_param: Rc<RefCell<Option<JsValue>>>,
    on_error: Callback<WebRtcMediaError>,
    media_generation: Rc<RefCell<u64>>,
    closed: Rc<RefCell<bool>>,
    _on_ice_candidate: Closure<dyn FnMut(RtcPeerConnectionIceEvent)>,
    _on_track: Closure<dyn FnMut(RtcTrackEvent)>,
}

impl WebRtcRoomSession {
    /// Synchronously wire up the `RTCPeerConnection` and kick off the async
    /// getUserMedia + offer flow inside `spawn_local`.
    pub fn start(
        transport: SignalingTransport,
        mic_enabled: bool,
        camera_enabled: bool,
        mic_device_id: String,
        camera_device_id: String,
        input_level: u32,
        on_streams: Callback<HashMap<String, RemoteParticipantMedia>>,
        on_error: Callback<WebRtcMediaError>,
    ) -> Result<Self, String> {
        let pc = create_peer_connection()?;
        let signaling = transport.clone();
        let local_stream: Rc<RefCell<Option<MediaStream>>> = Rc::default();
        let screen_stream: Rc<RefCell<Option<MediaStream>>> = Rc::default();
        let audio_sender: Rc<RefCell<Option<JsValue>>> =
            Rc::new(RefCell::new(add_transceiver_sender(&pc, "audio")));
        let video_sender: Rc<RefCell<Option<JsValue>>> =
            Rc::new(RefCell::new(add_transceiver_sender(&pc, "video")));
        let screen_sender: Rc<RefCell<Option<JsValue>>> =
            Rc::new(RefCell::new(add_transceiver_sender(&pc, "video")));
        let mic_audio_ctx: Rc<RefCell<Option<JsValue>>> = Rc::default();
        let mic_gain_param: Rc<RefCell<Option<JsValue>>> = Rc::default();
        let media_generation: Rc<RefCell<u64>> = Rc::default();
        let closed: Rc<RefCell<bool>> = Rc::default();

        // ── onicecandidate ───────────────────────────────────────────────────
        let transport_ice = transport.clone();
        let on_ice = Closure::wrap(Box::new(move |event: RtcPeerConnectionIceEvent| {
            let Some(candidate) = event.candidate() else {
                return;
            };
            let c = candidate.candidate();
            let mid = candidate.sdp_mid().unwrap_or_default();
            // Server validation rejects empty candidate or sdp_mid.
            if !c.is_empty() && !mid.is_empty() {
                let _ = transport_ice.send_ice_candidate(c, mid);
            }
        }) as Box<dyn FnMut(RtcPeerConnectionIceEvent)>);
        pc.set_onicecandidate(Some(on_ice.as_ref().unchecked_ref()));

        // ── ontrack ──────────────────────────────────────────────────────────
        // One composite MediaStream per remote publisher (audio + video on same stream).
        let streams_map: Rc<RefCell<HashMap<String, RemoteParticipantMedia>>> = Rc::default();
        let streams_map_cb = streams_map.clone();
        let on_track_error = on_error.clone();
        let on_track = Closure::wrap(Box::new(move |event: RtcTrackEvent| {
            let Ok(track) = event.track().dyn_into::<MediaStreamTrack>() else {
                return;
            };
            let Some((publisher, source)) = remote_source_from_relay_track_id(&track.id()) else {
                emit_error(
                    &on_track_error,
                    WebRtcMediaSeverity::Warning,
                    "remote_track",
                    "Received a remote media track that could not be assigned to a participant.",
                    format!("unparseable relay track id: {}", track.id()),
                );
                return;
            };
            let snapshot = {
                let mut map = streams_map_cb.borrow_mut();
                let entry = map
                    .entry(publisher.clone())
                    .or_insert_with(RemoteParticipantMedia::empty);
                let slot = if source == RemoteSource::Screen {
                    &mut entry.screen
                } else {
                    &mut entry.camera
                };
                if slot.is_none() {
                    let Ok(stream) = MediaStream::new() else {
                        return;
                    };
                    *slot = Some(JsValue::from(stream));
                }
                let Some(js) = slot.as_ref() else {
                    return;
                };
                let Ok(stream) = js.clone().dyn_into::<MediaStream>() else {
                    return;
                };
                stream.add_track(&track);
                map.clone()
            };
            on_streams.emit(snapshot);
        }) as Box<dyn FnMut(RtcTrackEvent)>);
        pc.set_ontrack(Some(on_track.as_ref().unchecked_ref()));

        // ── async: getUserMedia → add tracks → createOffer ───────────────────
        {
            let pc = pc.clone();
            let transport = transport.clone();
            let local_stream = local_stream.clone();
            let audio_sender = audio_sender.clone();
            let video_sender = video_sender.clone();
            let mic_audio_ctx = mic_audio_ctx.clone();
            let mic_gain_param = mic_gain_param.clone();
            let on_error = on_error.clone();
            let closed = closed.clone();
            let media_generation_async = media_generation.clone();
            let generation = next_generation(&media_generation);
            spawn_local(async move {
                if is_stale(&closed, &media_generation_async, generation) {
                    return;
                }
                match get_user_media(
                    mic_enabled,
                    camera_enabled,
                    &mic_device_id,
                    &camera_device_id,
                )
                .await
                {
                    Ok(stream) => {
                        if is_stale(&closed, &media_generation_async, generation) {
                            stop_all_tracks(&stream);
                            return;
                        }
                        let stream_for_pc = process_local_media_for_send(
                            &stream,
                            input_level,
                            &mic_audio_ctx,
                            &mic_gain_param,
                        )
                        .await;
                        let tracks = stream_for_pc.get_tracks();
                        for i in 0..tracks.length() {
                            if let Ok(track) = tracks.get(i).dyn_into::<MediaStreamTrack>() {
                                match track.kind().as_str() {
                                    "audio" => {
                                        if let Some(sender) = audio_sender.borrow().as_ref() {
                                            let _ = sender_replace_track(sender, Some(&track)).await;
                                        } else if let Some(sender_js) =
                                            rtc_add_track_return(&pc, &track, &stream_for_pc)
                                        {
                                            *audio_sender.borrow_mut() = Some(sender_js);
                                        }
                                    }
                                    "video" => {
                                        if let Some(sender) = video_sender.borrow().as_ref() {
                                            let _ = sender_replace_track(sender, Some(&track)).await;
                                        } else if let Some(sender_js) =
                                            rtc_add_track_return(&pc, &track, &stream_for_pc)
                                        {
                                            *video_sender.borrow_mut() = Some(sender_js);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        *local_stream.borrow_mut() = Some(stream_for_pc);
                    }
                    Err(err) => {
                        // Proceed without local media; we can still receive remote streams.
                        emit_error(
                            &on_error,
                            WebRtcMediaSeverity::Warning,
                            "get_user_media",
                            "Microphone or camera could not be started. The call will continue without local media.",
                            js_error_to_string(err),
                        );
                    }
                }

                if let Err(err) = send_local_offer(&pc, &transport).await {
                    emit_error(
                        &on_error,
                        WebRtcMediaSeverity::Fatal,
                        "create_offer",
                        "Could not start WebRTC media negotiation.",
                        err,
                    );
                }
            });
        }

        Ok(Self {
            pc,
            signaling,
            local_stream,
            screen_stream,
            audio_sender,
            video_sender,
            screen_sender,
            mic_audio_ctx,
            mic_gain_param,
            on_error,
            media_generation,
            closed,
            _on_ice_candidate: on_ice,
            _on_track: on_track,
        })
    }

    /// Re-acquire local media with updated settings and swap tracks on the peer connection.
    ///
    /// Uses `replaceTrack` when a sender already exists, `addTrack` (+ follow-up offer) when
    /// enabling a kind that had no sender. Calls `on_updated` after local preview state changes
    /// so the UI can re-render.
    pub fn apply_local_media(
        &self,
        mic_enabled: bool,
        camera_enabled: bool,
        mic_device_id: String,
        camera_device_id: String,
        input_level: u32,
        on_updated: Callback<()>,
    ) {
        let pc = self.pc.clone();
        let signaling = self.signaling.clone();
        let local_stream = self.local_stream.clone();
        let audio_sender = self.audio_sender.clone();
        let video_sender = self.video_sender.clone();
        let mic_audio_ctx = self.mic_audio_ctx.clone();
        let mic_gain_param = self.mic_gain_param.clone();
        let on_error = self.on_error.clone();
        let closed = self.closed.clone();
        let media_generation = self.media_generation.clone();
        let generation = next_generation(&media_generation);
        spawn_local(async move {
            if is_stale(&closed, &media_generation, generation) {
                return;
            }
            if !mic_enabled && !camera_enabled {
                teardown_mic_graph(&mic_audio_ctx, &mic_gain_param);
                if let Some(sj) = audio_sender.borrow().as_ref() {
                    if let Err(err) = sender_replace_track(sj, None).await {
                        emit_error(
                            &on_error,
                            WebRtcMediaSeverity::Recoverable,
                            "replace_audio_track",
                            "Could not mute microphone on the active WebRTC sender.",
                            err,
                        );
                    }
                }
                if let Some(sj) = video_sender.borrow().as_ref() {
                    if let Err(err) = sender_replace_track(sj, None).await {
                        emit_error(
                            &on_error,
                            WebRtcMediaSeverity::Recoverable,
                            "replace_video_track",
                            "Could not turn off camera on the active WebRTC sender.",
                            err,
                        );
                    }
                }
                if let Some(old) = local_stream.borrow_mut().take() {
                    stop_all_tracks(&old);
                }
                on_updated.emit(());
                return;
            }

            let stream = match get_user_media(
                mic_enabled,
                camera_enabled,
                &mic_device_id,
                &camera_device_id,
            )
            .await
            {
                Ok(s) => s,
                Err(err) => {
                    emit_error(
                        &on_error,
                        WebRtcMediaSeverity::Recoverable,
                        "get_user_media",
                        "Could not apply the selected microphone or camera settings.",
                        js_error_to_string(err),
                    );
                    on_updated.emit(());
                    return;
                }
            };
            if is_stale(&closed, &media_generation, generation) {
                stop_all_tracks(&stream);
                return;
            }

            let stream_for_pc = process_local_media_for_send(
                &stream,
                input_level,
                &mic_audio_ctx,
                &mic_gain_param,
            )
            .await;

            let audio_track = first_track_kind(&stream_for_pc, "audio");
            let video_track = first_track_kind(&stream_for_pc, "video");
            let mut needs_renegotiation = false;

            if mic_enabled {
                if let Some(ref t) = audio_track {
                    if let Some(sj) = audio_sender.borrow().as_ref() {
                        if let Err(err) = sender_replace_track(sj, Some(t)).await {
                            emit_error(
                                &on_error,
                                WebRtcMediaSeverity::Recoverable,
                                "replace_audio_track",
                                "Could not switch microphone on the active WebRTC sender.",
                                err,
                            );
                            stop_all_tracks(&stream_for_pc);
                            on_updated.emit(());
                            return;
                        }
                    } else if let Some(sender_js) =
                        rtc_add_track_return(&pc, t, &stream_for_pc)
                    {
                        *audio_sender.borrow_mut() = Some(sender_js);
                        needs_renegotiation = true;
                    }
                }
            } else if let Some(sj) = audio_sender.borrow().as_ref() {
                if let Err(err) = sender_replace_track(sj, None).await {
                    emit_error(
                        &on_error,
                        WebRtcMediaSeverity::Recoverable,
                        "replace_audio_track",
                        "Could not mute microphone on the active WebRTC sender.",
                        err,
                    );
                }
            }

            if camera_enabled {
                if let Some(ref t) = video_track {
                    if let Some(sj) = video_sender.borrow().as_ref() {
                        if let Err(err) = sender_replace_track(sj, Some(t)).await {
                            emit_error(
                                &on_error,
                                WebRtcMediaSeverity::Recoverable,
                                "replace_video_track",
                                "Could not switch camera on the active WebRTC sender.",
                                err,
                            );
                            stop_all_tracks(&stream_for_pc);
                            on_updated.emit(());
                            return;
                        }
                    } else if let Some(sender_js) =
                        rtc_add_track_return(&pc, t, &stream_for_pc)
                    {
                        *video_sender.borrow_mut() = Some(sender_js);
                        needs_renegotiation = true;
                    }
                }
            } else if let Some(sj) = video_sender.borrow().as_ref() {
                if let Err(err) = sender_replace_track(sj, None).await {
                    emit_error(
                        &on_error,
                        WebRtcMediaSeverity::Recoverable,
                        "replace_video_track",
                        "Could not turn off camera on the active WebRTC sender.",
                        err,
                    );
                }
            }

            if needs_renegotiation {
                if let Err(err) = send_local_offer(&pc, &signaling).await {
                    emit_error(
                        &on_error,
                        WebRtcMediaSeverity::Fatal,
                        "create_offer",
                        "Could not renegotiate WebRTC media after changing devices.",
                        err,
                    );
                    stop_all_tracks(&stream_for_pc);
                    on_updated.emit(());
                    return;
                }
            }

            if let Some(old) = local_stream.borrow_mut().take() {
                stop_all_tracks(&old);
            }
            *local_stream.borrow_mut() = Some(stream_for_pc);
            on_updated.emit(());
        });
    }

    /// Live mic gain (`0`…`100` → linear `0.0`…`1.0` on the [`GainNode`]).
    pub fn set_input_level(&self, input_level: u32) {
        let g = level_to_gain(input_level);
        if let Some(param) = self.mic_gain_param.borrow().as_ref() {
            let _ = Reflect::set(
                param,
                &JsValue::from_str("value"),
                &JsValue::from_f64(g),
            );
        }
    }

    pub fn publish_screen_share(&self, stream: JsValue, on_updated: Callback<()>) {
        let Ok(stream) = stream.dyn_into::<MediaStream>() else {
            emit_error(
                &self.on_error,
                WebRtcMediaSeverity::Recoverable,
                "screen_share",
                "Screen sharing stream could not be attached to the call.",
                "provided value is not a MediaStream".to_owned(),
            );
            return;
        };
        let Some(track) = first_track_kind(&stream, "video") else {
            emit_error(
                &self.on_error,
                WebRtcMediaSeverity::Recoverable,
                "screen_share",
                "Screen sharing did not provide a video track.",
                "display MediaStream has no video track".to_owned(),
            );
            return;
        };

        let pc = self.pc.clone();
        let signaling = self.signaling.clone();
        let screen_sender = self.screen_sender.clone();
        let screen_stream = self.screen_stream.clone();
        let on_error = self.on_error.clone();
        let closed = self.closed.clone();
        let media_generation = self.media_generation.clone();
        let generation = next_generation(&media_generation);
        spawn_local(async move {
            if is_stale(&closed, &media_generation, generation) {
                stop_all_tracks(&stream);
                return;
            }

            let mut needs_offer = false;
            if let Some(sender) = screen_sender.borrow().as_ref() {
                if let Err(err) = sender_replace_track(sender, Some(&track)).await {
                    emit_error(
                        &on_error,
                        WebRtcMediaSeverity::Recoverable,
                        "screen_share_replace_track",
                        "Could not publish screen sharing to the active call.",
                        err,
                    );
                    stop_all_tracks(&stream);
                    return;
                }
            } else if let Some(sender) = rtc_add_track_return(&pc, &track, &stream) {
                *screen_sender.borrow_mut() = Some(sender);
                needs_offer = true;
            } else {
                emit_error(
                    &on_error,
                    WebRtcMediaSeverity::Recoverable,
                    "screen_share_add_track",
                    "Could not add screen sharing to the active call.",
                    "RTCPeerConnection.addTrack returned no sender".to_owned(),
                );
                stop_all_tracks(&stream);
                return;
            }

            if needs_offer {
                if let Err(err) = send_local_offer(&pc, &signaling).await {
                    emit_error(
                        &on_error,
                        WebRtcMediaSeverity::Fatal,
                        "screen_share_offer",
                        "Could not negotiate screen sharing with the SFU.",
                        err,
                    );
                    stop_all_tracks(&stream);
                    return;
                }
            }

            if let Some(old) = screen_stream.borrow_mut().take() {
                stop_all_tracks(&old);
            }
            *screen_stream.borrow_mut() = Some(stream);
            on_updated.emit(());
        });
    }

    pub fn stop_screen_share(&self, on_updated: Callback<()>) {
        let pc = self.pc.clone();
        let signaling = self.signaling.clone();
        let screen_sender = self.screen_sender.clone();
        let screen_stream = self.screen_stream.clone();
        let on_error = self.on_error.clone();
        let closed = self.closed.clone();
        let media_generation = self.media_generation.clone();
        let generation = next_generation(&media_generation);
        spawn_local(async move {
            if is_stale(&closed, &media_generation, generation) {
                return;
            }
            if let Some(sender) = screen_sender.borrow().as_ref() {
                if let Err(err) = sender_replace_track(sender, None).await {
                    emit_error(
                        &on_error,
                        WebRtcMediaSeverity::Recoverable,
                        "screen_share_stop",
                        "Could not stop screen sharing on the active call.",
                        err,
                    );
                }
            }
            if let Some(old) = screen_stream.borrow_mut().take() {
                stop_all_tracks(&old);
            }
            if let Err(err) = send_local_offer(&pc, &signaling).await {
                emit_error(
                    &on_error,
                    WebRtcMediaSeverity::Recoverable,
                    "screen_share_stop_offer",
                    "Screen sharing stopped locally, but SFU renegotiation failed.",
                    err,
                );
            }
            on_updated.emit(());
        });
    }

    /// Apply a remote SDP description (answer from the SFU).
    pub fn handle_remote_sdp(&self, sdp: String, sdp_type: String) {
        let pc = self.pc.clone();
        let on_error = self.on_error.clone();
        spawn_local(async move {
            let rtc_type = if sdp_type == "offer" {
                RtcSdpType::Offer
            } else {
                RtcSdpType::Answer
            };
            let desc = RtcSessionDescriptionInit::new(rtc_type);
            desc.set_sdp(&sdp);
            if let Err(err) = JsFuture::from(pc.set_remote_description(&desc)).await {
                emit_error(
                    &on_error,
                    WebRtcMediaSeverity::Fatal,
                    "set_remote_description",
                    "Could not apply the SFU WebRTC answer.",
                    js_error_to_string(err),
                );
            }
        });
    }

    /// Add a remote ICE candidate from the SFU.
    pub fn handle_remote_ice_candidate(&self, candidate: String, sdp_mid: String) {
        let pc = self.pc.clone();
        let on_error = self.on_error.clone();
        spawn_local(async move {
            // Build RTCIceCandidateInit as a plain JS object to avoid web-sys version
            // sensitivity around the dict constructor.
            let init = Object::new();
            let _ = Reflect::set(
                &init,
                &JsValue::from_str("candidate"),
                &JsValue::from_str(&candidate),
            );
            let _ = Reflect::set(
                &init,
                &JsValue::from_str("sdpMid"),
                &JsValue::from_str(&sdp_mid),
            );
            // addIceCandidate via reflection — binding name varies across web-sys releases.
            let add_fn = match Reflect::get(&JsValue::from(&pc), &JsValue::from_str("addIceCandidate")) {
                Ok(f) => f.unchecked_into::<js_sys::Function>(),
                Err(err) => {
                    emit_error(
                        &on_error,
                        WebRtcMediaSeverity::Recoverable,
                        "add_ice_candidate",
                        "Could not apply an SFU ICE candidate.",
                        js_error_to_string(err),
                    );
                    return;
                }
            };
            let args = Array::new();
            args.push(&init);
            match add_fn.apply(&JsValue::from(&pc), &args) {
                Ok(promise_js) => {
                    if let Ok(promise) = promise_js.dyn_into::<js_sys::Promise>() {
                        if let Err(err) = JsFuture::from(promise).await {
                            emit_error(
                                &on_error,
                                WebRtcMediaSeverity::Recoverable,
                                "add_ice_candidate",
                                "Could not apply an SFU ICE candidate.",
                                js_error_to_string(err),
                            );
                        }
                    }
                }
                Err(err) => emit_error(
                    &on_error,
                    WebRtcMediaSeverity::Recoverable,
                    "add_ice_candidate",
                    "Could not apply an SFU ICE candidate.",
                    js_error_to_string(err),
                ),
            }
        });
    }

    /// The local camera/mic `MediaStream` as a `JsValue`, once acquired.
    pub fn local_stream(&self) -> Option<JsValue> {
        self.local_stream
            .borrow()
            .as_ref()
            .map(|s| JsValue::from(s))
    }

    /// Stop local tracks and close the peer connection.
    pub fn close(&self) {
        *self.closed.borrow_mut() = true;
        {
            let mut generation = self.media_generation.borrow_mut();
            *generation = generation.saturating_add(1);
        }
        teardown_mic_graph(&self.mic_audio_ctx, &self.mic_gain_param);
        if let Some(stream) = self.local_stream.borrow().as_ref() {
            let tracks = stream.get_tracks();
            for i in 0..tracks.length() {
                if let Ok(track) = tracks.get(i).dyn_into::<MediaStreamTrack>() {
                    track.stop();
                }
            }
        }
        if let Some(stream) = self.screen_stream.borrow().as_ref() {
            stop_all_tracks(stream);
        }
        self.pc.close();
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemoteSource {
    Camera,
    Screen,
}

/// SFU relay track id: `{publisher_id}|{source_type}:{mid}` where `publisher_id` is
/// the participant nickname and `source_type` is currently `audio`, `camera`, or `screen`.
fn remote_source_from_relay_track_id(track_id: &str) -> Option<(String, RemoteSource)> {
    let (publisher, source_track_id) = track_id.split_once('|')?;
    let (source_type, _) = source_track_id.split_once(':')?;
    if publisher.is_empty() || source_type.is_empty() {
        return None;
    }
    let source = if source_type == "screen" {
        RemoteSource::Screen
    } else {
        RemoteSource::Camera
    };
    Some((publisher.to_owned(), source))
}

fn create_peer_connection() -> Result<RtcPeerConnection, String> {
    // Build RTCConfiguration as a plain JS object — more portable than typed
    // web-sys dict constructors whose signatures vary between versions.
    let config = Object::new();
    let ice_servers = Array::new();
    let server = Object::new();
    let _ = Reflect::set(
        &server,
        &JsValue::from_str("urls"),
        &JsValue::from_str("stun:stun.l.google.com:19302"),
    );
    ice_servers.push(&server);
    let _ = Reflect::set(&config, &JsValue::from_str("iceServers"), &ice_servers);
    RtcPeerConnection::new_with_configuration(&config.unchecked_into())
        .map_err(|e| format!("RTCPeerConnection: {e:?}"))
}

/// Pre-creates stable m-line slots expected by the SFU fixed-slot policy.
fn add_transceiver_sender(pc: &RtcPeerConnection, kind: &str) -> Option<JsValue> {
    let add_fn: js_sys::Function =
        Reflect::get(&JsValue::from(pc), &JsValue::from_str("addTransceiver"))
            .ok()?
            .unchecked_into();
    let init = Object::new();
    let _ = Reflect::set(
        &init,
        &JsValue::from_str("direction"),
        &JsValue::from_str("sendrecv"),
    );
    let transceiver = add_fn
        .call2(&JsValue::from(pc), &JsValue::from_str(kind), &init)
        .ok()?;
    Reflect::get(&transceiver, &JsValue::from_str("sender")).ok()
}

/// `pc.addTrack(track, stream)`; returns the [`RtcRtpSender`](https://developer.mozilla.org/en-US/docs/Web/API/RTCRtpSender) as [`JsValue`].
fn rtc_add_track_return(
    pc: &RtcPeerConnection,
    track: &MediaStreamTrack,
    stream: &MediaStream,
) -> Option<JsValue> {
    let add_fn: js_sys::Function = Reflect::get(&JsValue::from(pc), &JsValue::from_str("addTrack"))
        .ok()?
        .unchecked_into();
    let args = Array::new();
    args.push(track);
    args.push(stream);
    add_fn.apply(&JsValue::from(pc), &args).ok()
}

fn first_track_kind(stream: &MediaStream, kind: &str) -> Option<MediaStreamTrack> {
    let tracks = stream.get_tracks();
    for i in 0..tracks.length() {
        if let Ok(t) = tracks.get(i).dyn_into::<MediaStreamTrack>() {
            if t.kind().eq_ignore_ascii_case(kind) {
                return Some(t);
            }
        }
    }
    None
}

fn stop_all_tracks(stream: &MediaStream) {
    let tracks = stream.get_tracks();
    for i in 0..tracks.length() {
        if let Ok(track) = tracks.get(i).dyn_into::<MediaStreamTrack>() {
            track.stop();
        }
    }
}

fn level_to_gain(input_level: u32) -> f64 {
    (input_level.min(100) as f64 / 100.0).clamp(0.0, 1.0)
}

fn next_generation(cell: &Rc<RefCell<u64>>) -> u64 {
    let mut generation = cell.borrow_mut();
    *generation = generation.saturating_add(1);
    *generation
}

fn is_stale(closed: &Rc<RefCell<bool>>, generation_cell: &Rc<RefCell<u64>>, generation: u64) -> bool {
    *closed.borrow() || *generation_cell.borrow() != generation
}

fn emit_error(
    callback: &Callback<WebRtcMediaError>,
    severity: WebRtcMediaSeverity,
    stage: &'static str,
    message: impl Into<String>,
    technical: impl Into<String>,
) {
    let error = WebRtcMediaError {
        severity,
        stage,
        message: message.into(),
        technical: technical.into(),
    };
    log_media_error(&error);
    callback.emit(error);
}

fn log_media_error(error: &WebRtcMediaError) {
    let text = format!(
        "WebRTC media error [{:?}] {}: {} ({})",
        error.severity, error.stage, error.message, error.technical
    );
    web_sys::console::error_1(&JsValue::from_str(&text));
}

fn js_error_to_string(value: JsValue) -> String {
    let name = Reflect::get(&value, &JsValue::from_str("name"))
        .ok()
        .and_then(|v| v.as_string());
    let message = Reflect::get(&value, &JsValue::from_str("message"))
        .ok()
        .and_then(|v| v.as_string())
        .or_else(|| value.as_string());
    match (name, message) {
        (Some(name), Some(message)) if !message.is_empty() => format!("{name}: {message}"),
        (Some(name), _) => name,
        (_, Some(message)) => message,
        _ => js_sys::JSON::stringify(&value)
            .ok()
            .and_then(|s| s.as_string())
            .unwrap_or_else(|| "JavaScript operation failed".to_owned()),
    }
}

fn teardown_mic_graph(ctx_cell: &RefCell<Option<JsValue>>, gain_cell: &RefCell<Option<JsValue>>) {
    *gain_cell.borrow_mut() = None;
    if let Some(ctx) = ctx_cell.borrow_mut().take() {
        if let Ok(close_m) = Reflect::get(&ctx, &JsValue::from_str("close")) {
            if let Ok(close_fn) = close_m.dyn_into::<js_sys::Function>() {
                let _ = close_fn.call0(&ctx);
            }
        }
    }
}

/// New stream with the same track objects (no processing).
fn copy_stream_tracks_to_new(raw: &MediaStream) -> Result<MediaStream, JsValue> {
    let out = MediaStream::new()?;
    let tracks = raw.get_tracks();
    for i in 0..tracks.length() {
        if let Ok(t) = tracks.get(i).dyn_into::<MediaStreamTrack>() {
            out.add_track(&t);
        }
    }
    Ok(out)
}

/// Routes mic audio through a [`GainNode`] so `input_level` affects what is sent on WebRTC.
/// Video tracks are passed through unchanged. Without an audio track, returns a shallow copy of `raw`.
async fn process_local_media_for_send(
    raw: &MediaStream,
    input_level: u32,
    ctx_cell: &RefCell<Option<JsValue>>,
    gain_cell: &RefCell<Option<JsValue>>,
) -> MediaStream {
    teardown_mic_graph(ctx_cell, gain_cell);

    if first_track_kind(raw, "audio").is_none() {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    }

    let window = match web_sys::window() {
        Some(w) => w,
        None => return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone()),
    };

    let ctx_ctor = match Reflect::get(&window, &JsValue::from_str("AudioContext"))
        .or_else(|_| Reflect::get(&window, &JsValue::from_str("webkitAudioContext")))
    {
        Ok(c) => c,
        Err(_) => return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone()),
    };
    let Ok(ctor) = ctx_ctor.dyn_into::<js_sys::Function>() else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };
    let Ok(ctx) = ctor.call0(&JsValue::undefined()) else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };

    if let Ok(resume_m) = Reflect::get(&ctx, &JsValue::from_str("resume")) {
        if let Ok(rf) = resume_m.dyn_into::<js_sys::Function>() {
            if let Ok(p) = rf.call0(&ctx) {
                if let Ok(promise) = p.dyn_into::<js_sys::Promise>() {
                    let _ = JsFuture::from(promise).await;
                }
            }
        }
    }

    let Ok(create_mss) = Reflect::get(&ctx, &JsValue::from_str("createMediaStreamSource")) else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };
    let Ok(create_mss) = create_mss.dyn_into::<js_sys::Function>() else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };
    let Ok(source) = create_mss.call1(&ctx, raw) else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };

    let Ok(create_gain) = Reflect::get(&ctx, &JsValue::from_str("createGain")) else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };
    let Ok(create_gain) = create_gain.dyn_into::<js_sys::Function>() else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };
    let Ok(gain_node) = create_gain.call0(&ctx) else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };

    let Ok(gain_param) = Reflect::get(&gain_node, &JsValue::from_str("gain")) else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };
    let g = level_to_gain(input_level);
    let _ = Reflect::set(
        &gain_param,
        &JsValue::from_str("value"),
        &JsValue::from_f64(g),
    );

    let Ok(create_dest) = Reflect::get(&ctx, &JsValue::from_str("createMediaStreamDestination"))
    else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };
    let Ok(create_dest) = create_dest.dyn_into::<js_sys::Function>() else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };
    let Ok(dest) = create_dest.call0(&ctx) else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };

    let Ok(conn1) = Reflect::get(&source, &JsValue::from_str("connect")) else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };
    let Ok(conn1) = conn1.dyn_into::<js_sys::Function>() else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };
    let _ = conn1.call1(&source, &gain_node);

    let Ok(conn2) = Reflect::get(&gain_node, &JsValue::from_str("connect")) else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };
    let Ok(conn2) = conn2.dyn_into::<js_sys::Function>() else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };
    let _ = conn2.call1(&gain_node, &dest);

    let Ok(dest_stream_js) = Reflect::get(&dest, &JsValue::from_str("stream")) else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };
    let Ok(dest_stream) = dest_stream_js.dyn_into::<MediaStream>() else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };

    let Ok(out) = MediaStream::new() else {
        return copy_stream_tracks_to_new(raw).unwrap_or_else(|_| raw.clone());
    };
    let audio_tracks = dest_stream.get_audio_tracks();
    for i in 0..audio_tracks.length() {
        if let Ok(t) = audio_tracks.get(i).dyn_into::<MediaStreamTrack>() {
            out.add_track(&t);
        }
    }
    let video_tracks = raw.get_video_tracks();
    for i in 0..video_tracks.length() {
        if let Ok(t) = video_tracks.get(i).dyn_into::<MediaStreamTrack>() {
            out.add_track(&t);
        }
    }

    *ctx_cell.borrow_mut() = Some(ctx);
    *gain_cell.borrow_mut() = Some(gain_param);

    out
}

async fn sender_replace_track(sender: &JsValue, track: Option<&MediaStreamTrack>) -> Result<(), String> {
    let Ok(rep_fn) = Reflect::get(sender, &JsValue::from_str("replaceTrack")) else {
        return Err("RTCRtpSender.replaceTrack is not available".to_owned());
    };
    let rep_fn: js_sys::Function = rep_fn.unchecked_into();
    let args = Array::new();
    match track {
        Some(t) => {
            args.push(t);
        }
        None => {
            args.push(&JsValue::NULL);
        }
    }
    let out = rep_fn.apply(sender, &args).map_err(js_error_to_string)?;
    if let Ok(promise) = out.dyn_into::<js_sys::Promise>() {
        JsFuture::from(promise).await.map_err(js_error_to_string)?;
    }
    Ok(())
}

async fn send_local_offer(pc: &RtcPeerConnection, transport: &SignalingTransport) -> Result<(), String> {
    let offer_js = JsFuture::from(pc.create_offer())
        .await
        .map_err(js_error_to_string)?;
    let sdp = Reflect::get(&offer_js, &JsValue::from_str("sdp"))
        .ok()
        .and_then(|v| v.as_string())
        .ok_or_else(|| "createOffer returned an SDP without text".to_owned())?;
    let sdp = rewrite_offer_mids_for_sfu(&sdp);
    let offer_init = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
    offer_init.set_sdp(&sdp);
    JsFuture::from(pc.set_local_description(&offer_init))
        .await
        .map_err(js_error_to_string)?;
    transport.send_sdp(sdp, "offer".to_string())
}

fn rewrite_offer_mids_for_sfu(sdp: &str) -> String {
    let normalized = sdp.replace("\r\n", "\n");
    let mut sections = normalized.split("\nm=").collect::<Vec<_>>();
    if sections.is_empty() {
        return sdp.to_owned();
    }

    let mut session = sections.remove(0).to_owned();
    let mut audio_seen = 0_usize;
    let mut video_seen = 0_usize;
    let mut mid_map = Vec::<(String, String)>::new();
    let rewritten_sections = sections
        .into_iter()
        .map(|section| {
            let full = format!("m={section}");
            if full.starts_with("m=audio") {
                audio_seen += 1;
                rewrite_section_mid(&full, "audio_main", &mut mid_map)
            } else if full.starts_with("m=video") {
                video_seen += 1;
                let mid = if video_seen == 1 {
                    "video_camera"
                } else {
                    "video_screen"
                };
                rewrite_section_mid(&full, mid, &mut mid_map)
            } else {
                full
            }
        })
        .collect::<Vec<_>>();

    session = rewrite_bundle_group(&session, &mid_map);
    let mut out = session;
    for section in rewritten_sections {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&section);
    }
    out.replace('\n', "\r\n")
}

fn rewrite_section_mid(
    section: &str,
    next_mid: &str,
    mid_map: &mut Vec<(String, String)>,
) -> String {
    section
        .lines()
        .map(|line| {
            if line.starts_with("a=mid:") {
                let old = line.trim_start_matches("a=mid:").to_owned();
                mid_map.push((old, next_mid.to_owned()));
                format!("a=mid:{next_mid}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn rewrite_bundle_group(session: &str, mid_map: &[(String, String)]) -> String {
    session
        .lines()
        .map(|line| {
            if !line.starts_with("a=group:BUNDLE") {
                return line.to_owned();
            }
            let mids = mid_map
                .iter()
                .map(|(_, next)| next.as_str())
                .collect::<Vec<_>>();
            if mids.is_empty() {
                line.to_owned()
            } else {
                format!("a=group:BUNDLE {}", mids.join(" "))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `getUserMedia` constraint for one kind: `false`, `true` (browser default), or
/// `{ deviceId: "<id>" }` when the user picked a specific device in settings.
fn media_track_constraint(enabled: bool, device_id: &str) -> JsValue {
    if !enabled {
        return JsValue::FALSE;
    }
    let id = device_id.trim();
    if id.is_empty() || id.eq_ignore_ascii_case("default") {
        return JsValue::TRUE;
    }
    let obj = Object::new();
    let _ = Reflect::set(
        &obj,
        &JsValue::from_str("deviceId"),
        &JsValue::from_str(id),
    );
    obj.into()
}

async fn get_user_media(
    mic_enabled: bool,
    camera_enabled: bool,
    mic_device_id: &str,
    camera_device_id: &str,
) -> Result<MediaStream, JsValue> {
    if !mic_enabled && !camera_enabled {
        return Err(JsValue::from_str("no media requested"));
    }
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let devices = window.navigator().media_devices()?;
    let constraints = MediaStreamConstraints::new();
    constraints.set_audio(&media_track_constraint(mic_enabled, mic_device_id));
    constraints.set_video(&media_track_constraint(
        camera_enabled,
        camera_device_id,
    ));
    let promise = devices.get_user_media_with_constraints(&constraints)?;
    let stream_js = JsFuture::from(promise).await?;
    stream_js.dyn_into::<MediaStream>()
}
