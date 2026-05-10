//! Modal to pick a **display** (monitor) before starting capture — not individual windows.

use crate::screen_share_pick::ScreenShareSource;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct ScreenShareDialogProps {
    pub sources: Vec<ScreenShareSource>,
    pub loading: bool,
    pub on_share: Callback<String>,
    pub on_close: Callback<()>,
}

#[function_component]
pub fn ScreenShareDialog(props: &ScreenShareDialogProps) -> Html {
    let selected: UseStateHandle<Option<usize>> = use_state(|| None);

    {
        let selected = selected.clone();
        let sources_key = props.sources.clone();
        use_effect_with(sources_key, move |_| {
            selected.set(None);
            || {}
        });
    }

    let on_pick = {
        let selected = selected.clone();
        Callback::from(move |idx: usize| {
            selected.set(if *selected == Some(idx) {
                None
            } else {
                Some(idx)
            });
        })
    };

    let on_share_click = {
        let selected = selected.clone();
        let sources = props.sources.clone();
        let on_share = props.on_share.clone();
        Callback::from(move |_| {
            if let Some(i) = *selected {
                if let Some(s) = sources.get(i) {
                    on_share.emit(s.id.clone());
                }
            }
        })
    };

    let status_text = if props.loading {
        "Loading displays…".to_string()
    } else if props.sources.is_empty() {
        "No screens detected".to_string()
    } else if let Some(i) = *selected {
        format!(
            "Selected: {} — press Share to start",
            props.sources.get(i).map(|s| s.label.as_str()).unwrap_or("")
        )
    } else {
        "Pick a display below, then press Share.".to_string()
    };

    let share_disabled = props.loading || props.sources.is_empty() || selected.is_none();

    html! {
        <div class="screen-share-dialog" role="presentation">
            <button
                type="button"
                class="screen-share-dialog__backdrop"
                aria-label="Close screen share dialog"
                onclick={props.on_close.reform(|_| ())}
            />
            <div
                class="screen-share-dialog__panel font-manrope text-on-background"
                role="dialog"
                aria-modal="true"
                aria-labelledby="screen-share-dialog-title"
            >
                <h2 id="screen-share-dialog-title" class="screen-share-dialog__title">
                    { "Share your screen" }
                </h2>
                <p class="screen-share-dialog__status" aria-live="polite">
                    { status_text }
                </p>

                <div class="screen-share-dialog__scroll">
                    if props.loading {
                        <div class="screen-share-dialog__empty">
                            { "Loading…" }
                        </div>
                    } else if props.sources.is_empty() {
                        <div class="screen-share-dialog__empty">
                            { "No capture targets available." }
                        </div>
                    } else {
                        <div class="screen-share-dialog__grid">
                            { for props.sources.iter().enumerate().map(|(idx, src)| {
                                let is_sel = *selected == Some(idx);
                                let on_pick = on_pick.clone();
                                let label = format!(
                                    "{} · {}×{}",
                                    src.label, src.width_px, src.height_px
                                );
                                let thumb_style = thumb_preview_style(src.width_px, src.height_px);
                                html! {
                                    <button
                                        type="button"
                                        class={classes!(
                                            "screen-share-dialog__tile",
                                            is_sel.then_some("screen-share-dialog__tile--selected")
                                        )}
                                        aria-pressed={is_sel.to_string()}
                                        onclick={Callback::from(move |_| on_pick.emit(idx))}
                                    >
                                        <div class="screen-share-dialog__thumb-wrap" aria-hidden="true">
                                            <div class="screen-share-dialog__thumb" style={thumb_style}>
                                                <div class="screen-share-dialog__thumb-inner" />
                                            </div>
                                        </div>
                                        <span class="screen-share-dialog__tile-label">{ label }</span>
                                    </button>
                                }
                            }) }
                        </div>
                    }
                </div>

                <div class="screen-share-dialog__actions">
                    <button
                        type="button"
                        class="screen-share-dialog__btn screen-share-dialog__btn--primary"
                        disabled={share_disabled}
                        onclick={on_share_click}
                    >
                        { "Share screen" }
                    </button>
                    <button
                        type="button"
                        class="screen-share-dialog__btn screen-share-dialog__btn--ghost"
                        onclick={props.on_close.reform(|_| ())}
                    >
                        { "Close" }
                    </button>
                </div>
            </div>
        </div>
    }
}

/// Preview keeps the monitor aspect ratio; width caps height (~220px) for portrait and wide layouts.
fn thumb_preview_style(width_px: u32, height_px: u32) -> String {
    let w = width_px.max(1);
    let h = height_px.max(1);
    format!("width: min(100%, calc(220px * {w} / {h})); aspect-ratio: {w} / {h}; height: auto;",)
}
