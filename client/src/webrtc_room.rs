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
//! 6. `ontrack` events populate `remote_streams` keyed by JS `MediaStream.id`, and the
//!    provided `on_streams` callback notifies the UI.
//! 7. `close()` stops local tracks and closes the PC.

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

pub struct WebRtcRoomSession {
    pc: RtcPeerConnection,
    local_stream: Rc<RefCell<Option<MediaStream>>>,
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
        on_streams: Callback<HashMap<String, JsValue>>,
    ) -> Result<Self, String> {
        let pc = create_peer_connection()?;
        let local_stream: Rc<RefCell<Option<MediaStream>>> = Rc::default();

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
        // Collect remote streams keyed by MediaStream.id. The UI maps these
        // to participant slots by arrival order (see app.rs participant_media).
        let streams_map: Rc<RefCell<HashMap<String, JsValue>>> = Rc::default();
        let streams_map_cb = streams_map.clone();
        let on_track = Closure::wrap(Box::new(move |event: RtcTrackEvent| {
            let js_streams = event.streams();
            if js_streams.length() == 0 {
                return;
            }
            let stream_js = js_streams.get(0);
            if let Ok(stream) = stream_js.clone().dyn_into::<MediaStream>() {
                let id = stream.id();
                streams_map_cb.borrow_mut().insert(id, stream_js);
                on_streams.emit(streams_map_cb.borrow().clone());
            }
        }) as Box<dyn FnMut(RtcTrackEvent)>);
        pc.set_ontrack(Some(on_track.as_ref().unchecked_ref()));

        // ── async: getUserMedia → add tracks → createOffer ───────────────────
        {
            let pc = pc.clone();
            let transport = transport.clone();
            let local_stream = local_stream.clone();
            spawn_local(async move {
                match get_user_media(mic_enabled, camera_enabled, &mic_device_id, &camera_device_id)
                    .await
                {
                    Ok(stream) => {
                        let tracks = stream.get_tracks();
                        for i in 0..tracks.length() {
                            if let Ok(track) = tracks.get(i).dyn_into::<MediaStreamTrack>() {
                                rtc_add_track(&pc, &track, &stream);
                            }
                        }
                        *local_stream.borrow_mut() = Some(stream);
                    }
                    Err(_) => {
                        // Proceed without local media; we can still receive remote streams.
                    }
                }

                let offer_js = match JsFuture::from(pc.create_offer()).await {
                    Ok(v) => v,
                    Err(_) => return,
                };
                let sdp = match Reflect::get(&offer_js, &JsValue::from_str("sdp"))
                    .ok()
                    .and_then(|v| v.as_string())
                {
                    Some(s) => s,
                    None => return,
                };
                let offer_init: &RtcSessionDescriptionInit = offer_js.unchecked_ref();
                if JsFuture::from(pc.set_local_description(offer_init))
                    .await
                    .is_err()
                {
                    return;
                }
                let _ = transport.send_sdp(sdp, "offer".to_string());
            });
        }

        Ok(Self {
            pc,
            local_stream,
            _on_ice_candidate: on_ice,
            _on_track: on_track,
        })
    }

    /// Apply a remote SDP description (answer from the SFU).
    pub fn handle_remote_sdp(&self, sdp: String, sdp_type: String) {
        let pc = self.pc.clone();
        spawn_local(async move {
            let rtc_type = if sdp_type == "offer" {
                RtcSdpType::Offer
            } else {
                RtcSdpType::Answer
            };
            let desc = RtcSessionDescriptionInit::new(rtc_type);
            desc.set_sdp(&sdp);
            let _ = JsFuture::from(pc.set_remote_description(&desc)).await;
        });
    }

    /// Add a remote ICE candidate from the SFU.
    pub fn handle_remote_ice_candidate(&self, candidate: String, sdp_mid: String) {
        let pc = self.pc.clone();
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
                Err(_) => return,
            };
            let args = Array::new();
            args.push(&init);
            if let Ok(promise_js) = add_fn.apply(&JsValue::from(&pc), &args) {
                if let Ok(promise) = promise_js.dyn_into::<js_sys::Promise>() {
                    let _ = JsFuture::from(promise).await;
                }
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
        if let Some(stream) = self.local_stream.borrow().as_ref() {
            let tracks = stream.get_tracks();
            for i in 0..tracks.length() {
                if let Ok(track) = tracks.get(i).dyn_into::<MediaStreamTrack>() {
                    track.stop();
                }
            }
        }
        self.pc.close();
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

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

/// Call `pc.addTrack(track, stream)` via JS reflection, avoiding web-sys
/// variadic-overload naming uncertainty.
fn rtc_add_track(pc: &RtcPeerConnection, track: &MediaStreamTrack, stream: &MediaStream) {
    let Ok(fn_val) = Reflect::get(&JsValue::from(pc), &JsValue::from_str("addTrack")) else {
        return;
    };
    let add_fn: js_sys::Function = fn_val.unchecked_into();
    let args = Array::new();
    args.push(track);
    args.push(stream);
    let _ = add_fn.apply(&JsValue::from(pc), &args);
}

async fn get_user_media(
    mic_enabled: bool,
    camera_enabled: bool,
    _mic_device_id: &str,
    _camera_device_id: &str,
) -> Result<MediaStream, JsValue> {
    if !mic_enabled && !camera_enabled {
        return Err(JsValue::from_str("no media requested"));
    }
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let devices = window.navigator().media_devices()?;
    let constraints = MediaStreamConstraints::new();
    constraints.set_audio(&JsValue::from(mic_enabled));
    constraints.set_video(&JsValue::from(camera_enabled));
    let promise = devices.get_user_media_with_constraints(&constraints)?;
    let stream_js = JsFuture::from(promise).await?;
    stream_js.dyn_into::<MediaStream>()
}
