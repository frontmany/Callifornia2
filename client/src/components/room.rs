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
use layout::{
    ParticipantLayout, TILE_ASPECT_W_OVER_H, TILE_GAP_PX,
    resolve_participant_layout, tiles_wrap_content_wh,
};
use participant_tile::ParticipantTile;
use tiles::{page_arrow_chevron, truncate_str};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue, UnwrapThrowExt};
use wasm_bindgen_futures::JsFuture;
use web_sys::window;
use yew::prelude::*;

// ---------------------------------------------------------------------------
// Stub data — replace once signaling provides real state.
// ---------------------------------------------------------------------------

/// Visible room-id length in the top chip; the full id is still copied.
const MAX_ROOM_ID_DISPLAY_CHARS: usize = 11;

// TODO: receive the real room id from join/create signaling response.
const DEMO_ROOM_ID: &str = "7f3a9c2b-demo-room";

// TODO: replace with the participant list delivered by signaling (joined/left events).
static MOCK_PARTICIPANTS: &[&str] = &["Alice", "Bob", "Charlie", "Dave", "Eve"];

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
    pub settings_state: SettingsState,
    pub on_theme_change: Callback<Theme>,
    pub on_toggle_mic: Callback<()>,
    pub on_toggle_camera: Callback<()>,
    pub on_input_level_change: Callback<u32>,
    pub on_output_level_change: Callback<u32>,
    pub on_mic_device_change: Callback<&'static str>,
    pub on_speaker_device_change: Callback<&'static str>,
    pub on_camera_device_change: Callback<&'static str>,
    pub on_end_call: Callback<()>,
    /// Call with `true` when the signaling WebSocket opens and `false` on close; parent stops connector `/session/renew` while `true`.
    #[prop_or_default]
    pub on_signaling_connected: Option<Callback<bool>>,
    /// Set to `true` when another participant is presenting; triggers the same
    /// screen-share layout as a local presentation.
    #[prop_or_default]
    pub remote_presentation_active: bool,
    /// Optional camera/preview stream per participant index (same order as your participant list).
    /// Pass each [`web_sys::MediaStream`] as [`JsValue`] (see [`media_stream_js`]).
    #[prop_or_default]
    pub participant_media: Vec<Option<JsValue>>,
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
                        wh_resize.set((w, h.max(280.0)));
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
                            wh_ro.set((w, h.max(280.0)));
                        }
                    },
                ) as Box<dyn FnMut(js_sys::Array, web_sys::ResizeObserver)>);
                let ro =
                    web_sys::ResizeObserver::new(ro_closure.as_ref().unchecked_ref()).unwrap_throw();
                ro.observe(&el);
                let (w0, h0) = tiles_wrap_content_wh(el.client_width(), el.client_height());
                if w0 > 0.0 {
                    tile_area_wh.set((w0, h0.max(280.0)));
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
                        wh_defer.set((w, h.max(280.0)));
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

    let participant_count = MOCK_PARTICIPANTS.len();
    let (vw, vh) = *tile_area_wh;
    let (layout, show_arrows_grid) = resolve_participant_layout(vw, vh, participant_count);
    let presentation = *screen_sharing || props.remote_presentation_active;

    let rail_page_count = participant_count
        .saturating_sub(1)
        .checked_div(RAIL_TILES_PER_PAGE)
        .map_or(1, |q| q + 1)
        .max(1);

    let page_count = if presentation { rail_page_count } else { layout.page_count };
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
        let open_screen_share_dialog = open_screen_share_dialog.clone();
        Callback::from(move |_| {
            if *screen_sharing {
                // TODO: stop the active MediaStream track here.
                screen_sharing.set(false);
            } else {
                open_screen_share_dialog.emit(());
            }
        })
    };

    let on_screen_share_confirm = {
        let screen_sharing = screen_sharing.clone();
        let screen_share_dialog_open = screen_share_dialog_open.clone();
        Callback::from(move |source_id: String| {
            let screen_sharing = screen_sharing.clone();
            let dialog_open = screen_share_dialog_open.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if screen_share_pick::start_system_screen_share_for_stream(&source_id)
                    .await
                    .is_ok()
                {
                    screen_sharing.set(true);
                    dialog_open.set(false);
                }
            });
        })
    };

    // ---------------------------------------------------------------------------
    // Room-id copy callbacks.
    // ---------------------------------------------------------------------------

    let on_copy_room_id = {
        let copy_flash = copy_flash.clone();
        Callback::from(move |_| {
            let copy_flash = copy_flash.clone();
            let room_id = DEMO_ROOM_ID.to_string();
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

    let room_id_display = truncate_str(DEMO_ROOM_ID, MAX_ROOM_ID_DISPLAY_CHARS);

    let tiles_band_w =
        layout.cols as f64 * layout.tile_width + (layout.cols.saturating_sub(1)) as f64 * TILE_GAP_PX;
    let tiles_band_h =
        layout.rows as f64 * layout.tile_height + (layout.rows.saturating_sub(1)) as f64 * TILE_GAP_PX;
    let tiles_cluster_style = cluster_style(tiles_band_w, tiles_band_h);

    let rail_tile_h = RAIL_TILE_W_PX / TILE_ASPECT_W_OVER_H;
    let rail_n = end.saturating_sub(start);
    let rail_band_h = rail_n as f64 * rail_tile_h + (rail_n.saturating_sub(1)) as f64 * TILE_GAP_PX;
    let rail_cluster_style = cluster_style(RAIL_TILE_W_PX, rail_band_h);

    // TODO: replace placeholder once the actual MediaStream is available.
    let stage_caption = if *screen_sharing { "Your screen" } else { "Shared screen" };

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
                    <span class="room-page__id-text" title={DEMO_ROOM_ID}>{ room_id_display }</span>
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
                            &MOCK_PARTICIPANTS[start..end],
                            start,
                            props.participant_media.as_slice(),
                            rail_cluster_style,
                            rail_tile_h,
                            show_arrows,
                            page,
                            page_count,
                            on_page_prev.clone(),
                            on_page_next.clone(),
                        ) }
                    } else {
                        { view_tiles(
                            &MOCK_PARTICIPANTS[start..end],
                            start,
                            props.participant_media.as_slice(),
                            &layout,
                            tiles_cluster_style,
                            show_arrows,
                            page,
                            on_page_prev.clone(),
                            on_page_next.clone(),
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

// ---------------------------------------------------------------------------
// Sub-view helpers.
// ---------------------------------------------------------------------------

/// Renders the normal participant grid with optional paging arrows.
#[allow(clippy::too_many_arguments)]
fn view_tiles(
    participants: &[&str],
    range_start: usize,
    participant_media: &[Option<JsValue>],
    layout: &ParticipantLayout,
    cluster_style: String,
    show_arrows: bool,
    page: usize,
    on_prev: Callback<()>,
    on_next: Callback<()>,
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
                        { for participants.iter().enumerate().map(|(i, &name)| {
                            let global_i = range_start + i;
                            let media = participant_media.get(global_i).cloned().flatten();
                            html! {
                                <ParticipantTile
                                    name={name.to_string()}
                                    media={media}
                                    width_px={layout.tile_width}
                                    height_px={layout.tile_height}
                                    rail={false}
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

/// Renders the screen-share layout: centred stage + paginated participant rail on the right.
#[allow(clippy::too_many_arguments)]
fn view_presentation(
    stage_caption: &str,
    participants: &[&str],
    range_start: usize,
    participant_media: &[Option<JsValue>],
    rail_cluster_style: String,
    rail_tile_h: f64,
    show_arrows: bool,
    page: usize,
    page_count: usize,
    on_prev: Callback<()>,
    on_next: Callback<()>,
) -> Html {
    html! {
        <div class="room-page__presentation-main">
            // ── Shared screen ────────────────────────────────────────────────
            <div class="room-page__presentation-stage" role="region" aria-label="Screen share">
                <div class="room-page__presentation-placeholder">
                    <span class="room-page__presentation-label">{ stage_caption }</span>
                    // TODO: replace with <video> bound to the active MediaStream.
                    <span class="room-page__presentation-hint">{ "Video will appear here" }</span>
                </div>
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
                            { for participants.iter().enumerate().map(|(i, &name)| {
                                let global_i = range_start + i;
                                let media = participant_media.get(global_i).cloned().flatten();
                                html! {
                                    <ParticipantTile
                                        name={name.to_string()}
                                        media={media}
                                        width_px={RAIL_TILE_W_PX}
                                        height_px={rail_tile_h}
                                        rail={true}
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
