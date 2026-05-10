//! In-call room view: participant tiles, media controls, room-id copy.

#[path = "room/layout.rs"]
mod layout;
#[path = "room/participant_tile.rs"]
pub mod participant_tile;
#[path = "room/tiles.rs"]
mod tiles;

use crate::app::SettingsState;
use crate::components::{ScreenShareDialog, SettingsPanel};
use crate::screen_share_pick::{self, ScreenShareSource};
use crate::theme::Theme;
use gloo_timers::future::TimeoutFuture;
use js_sys::Reflect;
use layout::{
    ParticipantLayout, TILE_ASPECT_W_OVER_H, TILE_GAP_PX, resolve_participant_layout,
    tiles_wrap_content_wh,
};
use participant_tile::ParticipantTile;
use tiles::{page_arrow_chevron, truncate_str};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue, UnwrapThrowExt};
use wasm_bindgen_futures::JsFuture;
use web_sys::{HtmlVideoElement, window};
use yew::prelude::*;

/// Visible room-id length in the top chip; the full id is still copied.
const MAX_ROOM_ID_DISPLAY_CHARS: usize = 11;

// ---------------------------------------------------------------------------
// Presentation-rail constants.
// ---------------------------------------------------------------------------

/// Fixed tile width for the compact right-rail in screen-share mode (16:9).
const RAIL_TILE_W_PX: f64 = 156.0;
/// Maximum participants shown per page in the presentation rail.
const RAIL_TILES_PER_PAGE: usize = 4;

// ---------------------------------------------------------------------------
// Component props.
// ---------------------------------------------------------------------------

#[derive(Properties, PartialEq)]
pub struct RoomProps {
    pub room_id: String,
    pub your_nickname: String,
    pub participants: Vec<String>,
    pub settings_state: SettingsState,
    pub on_theme_change: Callback<Theme>,
    pub on_toggle_mic: Callback<()>,
    pub on_toggle_camera: Callback<()>,
    pub on_input_level_change: Callback<u32>,
    pub on_output_level_change: Callback<u32>,
    pub on_mic_device_change: Callback<String>,
    pub on_speaker_device_change: Callback<String>,
    pub on_camera_device_change: Callback<String>,
    pub on_screen_share_started: Callback<JsValue>,
    pub on_screen_share_stopped: Callback<()>,
    pub on_screen_share_error: Callback<String>,
    pub on_media_error: Callback<String>,
    pub on_end_call: Callback<()>,
    /// Set to `true` when another participant is presenting; triggers the same
    /// screen-share layout as a local presentation.
    #[prop_or_default]
    pub remote_presentation_active: bool,
    /// Optional camera/preview stream per participant index (same order as your participant list).
    /// Pass each [`web_sys::MediaStream`] as [`JsValue`] (see [`media_stream_js`]).
    #[prop_or_default]
    pub participant_media: Vec<Option<JsValue>>,
    #[prop_or_default]
    pub remote_presentation_media: Option<JsValue>,
    #[prop_or_default]
    pub remote_presentation_owner: Option<String>,
    #[prop_or_default]
    pub media_error: Option<String>,
}

// ---------------------------------------------------------------------------
// Room component.
// ---------------------------------------------------------------------------

#[function_component]
pub fn Room(props: &RoomProps) -> Html {
    let is_settings_open = use_state(|| false);
    let screen_sharing = use_state(|| false);
    let screen_share_dialog_open = use_state(|| false);
    let screen_share_sources = use_state(|| Vec::<ScreenShareSource>::new());
    let screen_share_sources_loading = use_state(|| false);
    let screen_share_stream = use_state(|| Option::<JsValue>::None);
    let copy_flash = use_state(|| false);

    let tiles_wrap_ref = use_node_ref();
    let tile_area_wh = use_state(|| (0.0_f64, 0.0_f64));
    let page_index = use_state(|| 0_usize);

    // Track the tiles-wrap element size via ResizeObserver + window resize fallback.
    {
        let tiles_wrap_ref = tiles_wrap_ref.clone();
        let tile_area_wh = tile_area_wh.clone();
        use_effect_with((), move |_| {
            let window = web_sys::window().unwrap_throw();

            // Window-resize fallback (fires when ResizeObserver misses a change).
            let wh_resize = tile_area_wh.clone();
            let nr_resize = tiles_wrap_ref.clone();
            let resize_cb = Closure::wrap(Box::new(move || {
                if let Some(el) = nr_resize.cast::<web_sys::Element>() {
                    let (w, h) = tiles_wrap_content_wh(el.client_width(), el.client_height());
                    if w > 0.0 {
                        wh_resize.set((w, h));
                    }
                }
            }) as Box<dyn FnMut()>);
            window
                .add_event_listener_with_callback("resize", resize_cb.as_ref().unchecked_ref())
                .unwrap_throw();

            // Primary: ResizeObserver on the tiles-wrap element.
            let ro_pair = if let Some(el) = tiles_wrap_ref.cast::<web_sys::Element>() {
                let el_obs = el.clone();
                let wh_ro = tile_area_wh.clone();
                let ro_closure = Closure::wrap(Box::new(
                    move |_: js_sys::Array, _: web_sys::ResizeObserver| {
                        let (w, h) =
                            tiles_wrap_content_wh(el_obs.client_width(), el_obs.client_height());
                        if w > 0.0 {
                            wh_ro.set((w, h));
                        }
                    },
                )
                    as Box<dyn FnMut(js_sys::Array, web_sys::ResizeObserver)>);
                let ro = web_sys::ResizeObserver::new(ro_closure.as_ref().unchecked_ref())
                    .unwrap_throw();
                ro.observe(&el);
                let (w0, h0) = tiles_wrap_content_wh(el.client_width(), el.client_height());
                if w0 > 0.0 {
                    tile_area_wh.set((w0, h0));
                }
                Some((ro, ro_closure))
            } else {
                None
            };

            // Deferred initial read to catch sizes not yet available at mount time.
            let nr_defer = tiles_wrap_ref.clone();
            let wh_defer = tile_area_wh.clone();
            wasm_bindgen_futures::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(48).await;
                if let Some(el) = nr_defer.cast::<web_sys::Element>() {
                    let (w, h) = tiles_wrap_content_wh(el.client_width(), el.client_height());
                    if w > 0.0 {
                        wh_defer.set((w, h));
                    }
                }
            });

            move || {
                if let Some((ro, c)) = ro_pair {
                    ro.disconnect();
                    drop(c);
                }
                let _ = window.remove_event_listener_with_callback(
                    "resize",
                    resize_cb.as_ref().unchecked_ref(),
                );
                drop(resize_cb);
            }
        });
    }

    // ---------------------------------------------------------------------------
    // Pagination state.
    // ---------------------------------------------------------------------------

    let participant_count = props.participants.len();
    let (vw, vh) = *tile_area_wh;
    let (layout, show_arrows_grid) = resolve_participant_layout(vw, vh, participant_count);
    let presentation =
        *screen_sharing || props.remote_presentation_active || props.remote_presentation_media.is_some();

    let rail_page_count = participant_count
        .saturating_sub(1)
        .checked_div(RAIL_TILES_PER_PAGE)
        .map_or(1, |q| q + 1)
        .max(1);

    let page_count = if presentation {
        rail_page_count
    } else {
        layout.page_count
    };
    let page = (*page_index).min(page_count.saturating_sub(1));

    let (start, end, show_arrows) = if presentation {
        let s = page * RAIL_TILES_PER_PAGE;
        let e = (s + RAIL_TILES_PER_PAGE).min(participant_count);
        (s, e, rail_page_count > 1)
    } else {
        let s = page * layout.per_page;
        let e = (s + layout.per_page).min(participant_count);
        (s, e, show_arrows_grid)
    };

    // Clamp page index whenever the count or mode changes.
    {
        let page_index = page_index.clone();
        use_effect_with((presentation, page_count, vw, vh), move |(_, pc, _, _)| {
            let current = *page_index;
            let clamped = current.min(pc.saturating_sub(1));
            if clamped != current {
                page_index.set(clamped);
            }
        });
    }

    let on_page_prev = {
        let page_index = page_index.clone();
        Callback::from(move |_| page_index.set((*page_index).saturating_sub(1)))
    };
    let on_page_next = {
        let page_index = page_index.clone();
        let max_page = page_count.saturating_sub(1);
        Callback::from(move |_| page_index.set((*page_index + 1).min(max_page)))
    };

    // ---------------------------------------------------------------------------
    // Settings panel callbacks.
    // ---------------------------------------------------------------------------

    let open_settings = {
        let is_settings_open = is_settings_open.clone();
        Callback::from(move |_| is_settings_open.set(true))
    };
    let close_settings = {
        let is_settings_open = is_settings_open.clone();
        Callback::from(move |_| is_settings_open.set(false))
    };

    // ---------------------------------------------------------------------------
    // Screen-share callbacks.
    // ---------------------------------------------------------------------------

    let close_screen_share_dialog = {
        let screen_share_dialog_open = screen_share_dialog_open.clone();
        Callback::from(move |_| screen_share_dialog_open.set(false))
    };

    let open_screen_share_dialog = {
        let screen_share_dialog_open = screen_share_dialog_open.clone();
        let screen_share_sources = screen_share_sources.clone();
        let screen_share_sources_loading = screen_share_sources_loading.clone();
        Callback::from(move |_| {
            screen_share_dialog_open.set(true);
            screen_share_sources.set(Vec::new());
            screen_share_sources_loading.set(true);
            let sources = screen_share_sources.clone();
            let loading = screen_share_sources_loading.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let v = screen_share_pick::enumerate_screen_share_sources_for_dialog().await;
                sources.set(v);
                loading.set(false);
            });
        })
    };

    let on_toggle_screen = {
        let screen_sharing = screen_sharing.clone();
        let screen_share_stream = screen_share_stream.clone();
        let open_screen_share_dialog = open_screen_share_dialog.clone();
        let on_screen_share_stopped = props.on_screen_share_stopped.clone();
        Callback::from(move |_| {
            if *screen_sharing {
                if let Some(stream) = screen_share_stream.as_ref() {
                    screen_share_pick::stop_media_stream_tracks(stream);
                }
                on_screen_share_stopped.emit(());
                screen_share_stream.set(None);
                screen_sharing.set(false);
            } else {
                open_screen_share_dialog.emit(());
            }
        })
    };

    let on_screen_share_confirm = {
        let screen_sharing = screen_sharing.clone();
        let screen_share_stream = screen_share_stream.clone();
        let screen_share_dialog_open = screen_share_dialog_open.clone();
        let on_screen_share_started = props.on_screen_share_started.clone();
        let on_screen_share_error = props.on_screen_share_error.clone();
        Callback::from(move |source_id: String| {
            let screen_sharing = screen_sharing.clone();
            let screen_share_stream = screen_share_stream.clone();
            let dialog_open = screen_share_dialog_open.clone();
            let started = on_screen_share_started.clone();
            let error = on_screen_share_error.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match screen_share_pick::start_system_screen_share_for_stream(&source_id).await {
                    Ok(stream) => {
                        started.emit(stream.clone());
                        screen_share_stream.set(Some(stream));
                        screen_sharing.set(true);
                        dialog_open.set(false);
                    }
                    Err(err) => error.emit(err),
                }
            });
        })
    };

    // ---------------------------------------------------------------------------
    // Room-id copy callbacks.
    // ---------------------------------------------------------------------------

    let on_copy_room_id = {
        let copy_flash = copy_flash.clone();
        let room_id = props.room_id.clone();
        Callback::from(move |_| {
            let copy_flash = copy_flash.clone();
            let room_id = room_id.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if !copy_to_clipboard(&room_id).await {
                    return;
                }
                copy_flash.set(true);
                gloo_timers::future::TimeoutFuture::new(1_600).await;
                copy_flash.set(false);
            });
        })
    };

    // ---------------------------------------------------------------------------
    // Derived render values.
    // ---------------------------------------------------------------------------

    let room_id_display = truncate_str(&props.room_id, MAX_ROOM_ID_DISPLAY_CHARS);

    let tiles_band_w = layout.cols as f64 * layout.tile_width
        + (layout.cols.saturating_sub(1)) as f64 * TILE_GAP_PX;
    let tiles_band_h = layout.rows as f64 * layout.tile_height
        + (layout.rows.saturating_sub(1)) as f64 * TILE_GAP_PX;
    let tiles_cluster_style = cluster_style(tiles_band_w, tiles_band_h);

    let rail_tile_h = RAIL_TILE_W_PX / TILE_ASPECT_W_OVER_H;
    let rail_n = end.saturating_sub(start);
    let rail_band_h = rail_n as f64 * rail_tile_h + (rail_n.saturating_sub(1)) as f64 * TILE_GAP_PX;
    let rail_cluster_style = cluster_style(RAIL_TILE_W_PX, rail_band_h);

    let stage_caption = if *screen_sharing {
        "Your screen"
    } else if let Some(owner) = props.remote_presentation_owner.as_ref() {
        owner.as_str()
    } else {
        "Shared screen"
    };
    let stage_media = if *screen_sharing {
        (*screen_share_stream).clone()
    } else {
        props.remote_presentation_media.clone()
    };

    // Глобальная громкость динамиков: один коэффициент на все `<video>` в области звонка
    // (участники + screen share), без отдельной «громкости на участника».
    {
        let wrap = tiles_wrap_ref.clone();
        let output_level = props.settings_state.output_level;
        let speaker_device_id = props.settings_state.speaker_device_id.clone();
        let on_media_error = props.on_media_error.clone();
        let media_mask: Vec<bool> = props.participant_media.iter().map(|m| m.is_some()).collect();
        let screen_sharing_on = *screen_sharing;
        let screen_share_on = (*screen_share_stream).is_some();
        let remote_screen_on = props.remote_presentation_media.is_some();
        use_effect_with(
            (
                output_level,
                speaker_device_id,
                media_mask,
                screen_sharing_on,
                screen_share_on,
                remote_screen_on,
            ),
            move |(level, speaker, _, _, _, _)| {
                let wrap = wrap.clone();
                let level = *level;
                let speaker = speaker.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    TimeoutFuture::new(0).await;
                    if let Some(el) = wrap.cast::<web_sys::Element>() {
                        if let Some(err) =
                            apply_global_output_audio_settings(&el, level, &speaker).await
                        {
                            on_media_error.emit(err);
                        }
                    }
                });
                || {}
            },
        );
    }

    // ---------------------------------------------------------------------------
    // Render.
    // ---------------------------------------------------------------------------

    html! {
        <div class={classes!(
            "room-page",
            "font-manrope",
            "text-on-background",
            presentation.then_some("room-page--presentation")
        )}>
            // ── Room-id chip (top-left, always visible) ──────────────────────
            <div class="room-page__top-left">
                <div class="room-page__id-copy-chip">
                    <span class="room-page__id-text" title={props.room_id.clone()}>{ room_id_display }</span>
                    <button
                        type="button"
                        class="room-page__id-copy-hit"
                        onclick={on_copy_room_id.clone()}
                        aria-label="Copy room ID"
                    >
                        <img
                            src={if *copy_flash { "icons/copied.svg" } else { "icons/copy.svg" }}
                            alt=""
                            aria-hidden="true"
                            class="room-page__copy-icon"
                        />
                    </button>
                </div>
            </div>

            // ── Settings button (top-right, always visible) ───────────────────
            <div class="room-page__actions room-page__actions--floating">
                <button
                    type="button"
                    class="room-page__icon-btn room-page__icon-btn--settings"
                    aria-label="Settings"
                    aria-haspopup="dialog"
                    onclick={open_settings}
                >
                    { settings_gear_icon() }
                </button>
            </div>

            // ── Main content area ─────────────────────────────────────────────
            <main class="room-page__main">
                <div ref={tiles_wrap_ref.clone()} class="room-page__viewport-fill">
                    if presentation {
                        { view_presentation(
                            stage_caption,
                            &props.participants[start..end],
                            start,
                            props.participant_media.as_slice(),
                            stage_media.clone(),
                            rail_cluster_style,
                            rail_tile_h,
                            show_arrows,
                            page,
                            page_count,
                            on_page_prev.clone(),
                            on_page_next.clone(),
                            props.your_nickname.as_str(),
                        ) }
                    } else {
                        { view_tiles(
                            &props.participants[start..end],
                            start,
                            props.participant_media.as_slice(),
                            &layout,
                            tiles_cluster_style,
                            show_arrows,
                            page,
                            on_page_prev.clone(),
                            on_page_next.clone(),
                            props.your_nickname.as_str(),
                        ) }
                    }
                </div>

                // ── Call controls (horizontal row in normal mode;
                //                  vertical column on the left in presentation mode) ──
                <footer class={classes!(
                    "room-page__controls",
                    presentation.then_some("room-page__controls--broadcast-column")
                )} aria-label="Call controls">
                    <button
                        type="button"
                        class={classes!(
                            "settings-panel__quick-btn",
                            "room-page__control-btn",
                            if props.settings_state.mic_enabled {
                                "settings-panel__quick-btn--on"
                            } else {
                                "settings-panel__quick-btn--off"
                            }
                        )}
                        onclick={props.on_toggle_mic.reform(|_| ())}
                        aria-label={if props.settings_state.mic_enabled { "Mute microphone" } else { "Unmute microphone" }}
                        title={if props.settings_state.mic_enabled { "Microphone on" } else { "Microphone muted" }}
                    >
                        <img
                            class="settings-panel__quick-icon"
                            src={if props.settings_state.mic_enabled {
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
                            "room-page__control-btn",
                            if props.settings_state.camera_enabled {
                                "settings-panel__quick-btn--on"
                            } else {
                                "settings-panel__quick-btn--off"
                            }
                        )}
                        onclick={props.on_toggle_camera.reform(|_| ())}
                        aria-label={if props.settings_state.camera_enabled { "Turn camera off" } else { "Turn camera on" }}
                        title={if props.settings_state.camera_enabled { "Camera on" } else { "Camera off" }}
                    >
                        <img
                            class="settings-panel__quick-icon room-page__cam-icon"
                            src={if props.settings_state.camera_enabled {
                                "icons/camera.svg"
                            } else {
                                "icons/cameraDisabled.svg"
                            }}
                            alt=""
                            aria-hidden="true"
                        />
                    </button>

                    <button
                        type="button"
                        class={classes!(
                            "settings-panel__quick-btn",
                            "room-page__control-btn",
                            "room-page__control-btn--wide",
                            "room-page__screen-btn",
                            if *screen_sharing {
                                "room-page__screen-btn--active"
                            } else {
                                "room-page__screen-btn--inactive"
                            }
                        )}
                        onclick={on_toggle_screen}
                        aria-label={if *screen_sharing { "Stop sharing screen" } else { "Share screen" }}
                        title={if *screen_sharing { "Screen sharing on" } else { "Share screen" }}
                    >
                        <img
                            class="settings-panel__quick-icon room-page__screen-icon"
                            src="icons/screenShare.svg"
                            alt=""
                            aria-hidden="true"
                        />
                    </button>

                    <button
                        type="button"
                        class="room-page__end-btn"
                        onclick={props.on_end_call.reform(|_| ())}
                        aria-label="End call"
                        title="End call"
                    >
                        <img
                            class="room-page__end-icon"
                            src="icons/phone.svg"
                            alt=""
                            aria-hidden="true"
                        />
                    </button>
                </footer>
            </main>

            // ── Overlays ──────────────────────────────────────────────────────
            if let Some(error) = props.media_error.as_ref() {
                <div class="room-page__media-warning" role="status" aria-live="polite">
                    { error.clone() }
                </div>
            }

            if *screen_share_dialog_open {
                <ScreenShareDialog
                    sources={(*screen_share_sources).clone()}
                    loading={*screen_share_sources_loading}
                    on_share={on_screen_share_confirm}
                    on_close={close_screen_share_dialog.clone()}
                />
            }

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

// ---------------------------------------------------------------------------
// Sub-view helpers.
// ---------------------------------------------------------------------------

fn playback_volume(output_level: u32) -> f64 {
    (output_level.min(100) as f64 / 100.0).clamp(0.0, 1.0)
}

async fn apply_global_output_audio_settings(
    container: &web_sys::Element,
    output_level: u32,
    speaker_device_id: &str,
) -> Option<String> {
    let vol = playback_volume(output_level);
    let sink_id = speaker_device_id.trim();
    let Ok(list) = container.query_selector_all("video") else {
        return None;
    };
    let mut sink_error = None;
    for i in 0..list.length() {
        if let Some(node) = list.item(i) {
            if let Ok(video) = node.dyn_into::<HtmlVideoElement>() {
                let _ = Reflect::set(
                    &video,
                    &JsValue::from_str("volume"),
                    &JsValue::from_f64(vol),
                );
                if !sink_id.is_empty() && !sink_id.eq_ignore_ascii_case("default") {
                    if let Err(err) = set_media_sink_id(&video, sink_id).await {
                        sink_error.get_or_insert(err);
                    }
                }
            }
        }
    }
    sink_error
}

async fn set_media_sink_id(video: &HtmlVideoElement, sink_id: &str) -> Result<(), String> {
    let set_sink = Reflect::get(video, &JsValue::from_str("setSinkId"))
        .map_err(js_error_to_string)?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| "Audio output device selection is not supported by this browser.".to_owned())?;
    let out = set_sink
        .call1(video, &JsValue::from_str(sink_id))
        .map_err(js_error_to_string)?;
    if let Ok(promise) = out.dyn_into::<js_sys::Promise>() {
        JsFuture::from(promise).await.map_err(js_error_to_string)?;
    }
    Ok(())
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
        _ => "Browser media operation failed.".to_owned(),
    }
}

/// Renders the normal participant grid with optional paging arrows.
#[allow(clippy::too_many_arguments)]
fn view_tiles(
    participants: &[String],
    range_start: usize,
    participant_media: &[Option<JsValue>],
    layout: &ParticipantLayout,
    cluster_style: String,
    show_arrows: bool,
    page: usize,
    on_prev: Callback<()>,
    on_next: Callback<()>,
    your_nickname: &str,
) -> Html {
    html! {
        <section class="room-page__tiles-wrap" aria-label="Participants">
            <div class="room-page__tiles-strip">
                if show_arrows {
                    <button
                        type="button"
                        class="room-page__page-arrow"
                        disabled={page == 0}
                        aria-label="Previous participants"
                        onclick={on_prev.reform(|_| ())}
                    >
                        { page_arrow_chevron(true) }
                    </button>
                }
                <div class="room-page__tiles-viewport">
                    <div class="room-page__tiles" style={cluster_style}>
                        { for participants.iter().enumerate().map(|(i, name)| {
                            let global_i = range_start + i;
                            let media = participant_media.get(global_i).cloned().flatten();
                            let is_self = name.as_str() == your_nickname;
                            html! {
                                <ParticipantTile
                                    name={name.clone()}
                                    media={media}
                                    width_px={layout.tile_width}
                                    height_px={layout.tile_height}
                                    rail={false}
                                    mute_audio={is_self}
                                />
                            }
                        }) }
                    </div>
                </div>
                if show_arrows {
                    <button
                        type="button"
                        class="room-page__page-arrow"
                        disabled={page + 1 >= layout.page_count}
                        aria-label="Next participants"
                        onclick={on_next.reform(|_| ())}
                    >
                        { page_arrow_chevron(false) }
                    </button>
                }
            </div>
        </section>
    }
}

#[derive(Properties, PartialEq, Clone)]
struct PresentationStageProps {
    caption: String,
    #[prop_or_default]
    media: Option<JsValue>,
}

#[function_component]
fn PresentationStage(props: &PresentationStageProps) -> Html {
    let video_ref = use_node_ref();
    {
        let video_ref = video_ref.clone();
        let media = props.media.clone();
        use_effect_with(media, move |media: &Option<JsValue>| {
            if let Some(video_el) = video_ref.cast::<HtmlVideoElement>() {
                let stream = media
                    .as_ref()
                    .and_then(|value| value.clone().dyn_into::<web_sys::MediaStream>().ok());
                let _ = video_el.set_src_object(stream.as_ref());
            }
            || ()
        });
    }

    html! {
        <div class="room-page__presentation-placeholder">
            if props.media.is_some() {
                <video
                    ref={video_ref}
                    class="room-page__tile-video"
                    autoplay=true
                    playsinline=true
                    muted=true
                    aria-label={props.caption.clone()}
                />
            } else {
                <span class="room-page__presentation-label">{ props.caption.clone() }</span>
                <span class="room-page__presentation-hint">{ "Screen share will appear here" }</span>
            }
        </div>
    }
}

/// Renders the screen-share layout: centred stage + paginated participant rail on the right.
#[allow(clippy::too_many_arguments)]
fn view_presentation(
    stage_caption: &str,
    participants: &[String],
    range_start: usize,
    participant_media: &[Option<JsValue>],
    stage_media: Option<JsValue>,
    rail_cluster_style: String,
    rail_tile_h: f64,
    show_arrows: bool,
    page: usize,
    page_count: usize,
    on_prev: Callback<()>,
    on_next: Callback<()>,
    your_nickname: &str,
) -> Html {
    html! {
        <div class="room-page__presentation-main">
            // ── Shared screen ────────────────────────────────────────────────
            <div class="room-page__presentation-stage" role="region" aria-label="Screen share">
                <PresentationStage caption={stage_caption.to_owned()} media={stage_media} />
            </div>

            // ── Participant rail (right, absolute) ───────────────────────────
            <aside class="room-page__participants-rail" aria-label="Participants">
                <div class="room-page__participants-rail-strip">
                    if show_arrows {
                        <button
                            type="button"
                            class="room-page__page-arrow room-page__page-arrow--rail-prev"
                            disabled={page == 0}
                            aria-label="Previous participants"
                            onclick={on_prev.reform(|_| ())}
                        >
                            { page_arrow_chevron(true) }
                        </button>
                    }
                    <div class="room-page__participants-rail-viewport">
                        <div
                            class="room-page__participants-rail-tiles"
                            style={rail_cluster_style}
                        >
                            { for participants.iter().enumerate().map(|(i, name)| {
                                let global_i = range_start + i;
                                let media = participant_media.get(global_i).cloned().flatten();
                                let is_self = name.as_str() == your_nickname;
                                html! {
                                    <ParticipantTile
                                        name={name.clone()}
                                        media={media}
                                        width_px={RAIL_TILE_W_PX}
                                        height_px={rail_tile_h}
                                        rail={true}
                                        mute_audio={is_self}
                                    />
                                }
                            }) }
                        </div>
                    </div>
                    if show_arrows {
                        <button
                            type="button"
                            class="room-page__page-arrow room-page__page-arrow--rail-next"
                            disabled={page + 1 >= page_count}
                            aria-label="Next participants"
                            onclick={on_next.reform(|_| ())}
                        >
                            { page_arrow_chevron(false) }
                        </button>
                    }
                </div>
            </aside>
        </div>
    }
}

// ---------------------------------------------------------------------------
// Tiny render helpers.
// ---------------------------------------------------------------------------

/// Returns the inline style that pins a cluster div to an exact `w × h` size.
fn cluster_style(w: f64, h: f64) -> String {
    format!(
        "width:{w:.1}px;min-width:{w:.1}px;max-width:{w:.1}px;\
         min-height:{h:.1}px;max-height:{h:.1}px;"
    )
}

/// Settings-gear SVG icon (reused in the top-right action bar).
fn settings_gear_icon() -> Html {
    html! {
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
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 \
                     0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 \
                     0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 \
                     1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 \
                     0-1.51-1H3a2 2 0 0 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 \
                     0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 \
                     0 1.82.33h.01a1.65 1.65 0 0 0 .99-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 \
                     .99 1.51h.01a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 \
                     2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.01a1.65 1.65 0 0 0 1.51.99H21a2 2 0 \
                     0 1 0 4h-.09a1.65 1.65 0 0 0-1.51.99z" />
        </svg>
    }
}

// ---------------------------------------------------------------------------
// Async utilities.
// ---------------------------------------------------------------------------

async fn copy_to_clipboard(text: &str) -> bool {
    let Some(w) = window() else { return false };
    JsFuture::from(w.navigator().clipboard().write_text(text))
        .await
        .is_ok()
}
