//! Rendering helpers for participant tiles and paging chevrons.

use yew::prelude::*;

/// Maximum participant name length shown in a tile (Unicode scalar values).
pub const MAX_NAME_CHARS: usize = 32;

/// Truncates `s` to at most `max` Unicode scalar values, appending `…` when cut.
pub fn truncate_str(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Renders a single participant tile.
///
/// `rail` adds `.room-page__tile--rail` for the compact presentation-mode style.
pub fn participant_tile(name: &str, tile_w: f64, tile_h: f64, rail: bool) -> Html {
    let label = truncate_str(name, MAX_NAME_CHARS);
    let style = format!("width:{tile_w:.1}px;height:{tile_h:.1}px;flex:0 0 auto;");
    html! {
        <div
            class={classes!("room-page__tile", rail.then_some("room-page__tile--rail"))}
            style={style}
        >
            <span class="room-page__tile-name" title={name.to_owned()}>{ label }</span>
        </div>
    }
}

/// Renders the SVG chevron used inside a paging arrow button.
///
/// `left = true` → left-pointing (previous); `false` → right-pointing (next).
pub fn page_arrow_chevron(left: bool) -> Html {
    let d = if left { "M15 18L9 12L15 6" } else { "M9 18L15 12L9 6" };
    html! {
        <svg
            class="room-page__page-arrow-icon"
            xmlns="http://www.w3.org/2000/svg"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
        >
            <path d={d} />
        </svg>
    }
}
