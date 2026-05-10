//! Geometry helpers: participant tile sizing and grid layout computation.

/// Gap between tiles in CSS pixels.
pub const TILE_GAP_PX: f64 = 14.0;
/// Slight upscale inside each grid cell (still clamped to cell and 16:9).
const TILE_SIZE_BOOST: f64 = 1.05;
/// Minimum tile width; never go below this even when space is tight.
pub const TILE_MIN_W_PX: f64 = 160.0;
/// Soft upper caps — the grid cell is the real bound at runtime.
const TILE_MAX_W_PX: f64 = 960.0;
const TILE_MAX_H_PX: f64 = 720.0;
/// Must match the horizontal and vertical padding on `.room-page__tiles-wrap` in `room.css`.
pub const TILE_WRAP_PAD_X: f64 = 16.0;
pub const TILE_WRAP_PAD_Y: f64 = 24.0;
/// Tile aspect ratio: width / height = 16 / 9.
pub const TILE_ASPECT_W_OVER_H: f64 = 16.0 / 9.0;
/// Maximum columns and rows in the paged grid.
pub const GRID_COLS: usize = 3;
pub const GRID_ROWS: usize = 2;
/// Tiles per page when the participant list is paginated.
pub const TILES_PER_PAGE: usize = GRID_COLS * GRID_ROWS;
/// Width reserved for one paging-arrow button (including its gap to the viewport).
const ARROW_SLOT_PX: f64 = 48.0;
/// Penalizes extra rows when a flatter layout still gives reasonably sized tiles.
const ROW_COUNT_SCORE_PENALTY: f64 = 1.35;

/// Computed geometry for one page of the participant grid.
#[derive(Clone, Copy)]
pub struct ParticipantLayout {
    pub cols: usize,
    pub rows: usize,
    pub tile_width: f64,
    pub tile_height: f64,
    /// Always [`TILES_PER_PAGE`]; stored here so callers do not need the constant.
    pub per_page: usize,
    pub page_count: usize,
}

/// Largest 16:9 rectangle that fits inside `cell_w × cell_h`, respecting all min/max caps.
fn tile_size_16_9_in_cell(cell_w: f64, cell_h: f64) -> (f64, f64) {
    let cw = cell_w.max(1.0);
    let ch = cell_h.max(1.0);
    let max_tw = cw.min(TILE_MAX_W_PX);
    let max_th = ch.min(TILE_MAX_H_PX);

    let mut tw = max_tw.min(max_th * TILE_ASPECT_W_OVER_H);
    let mut th = tw / TILE_ASPECT_W_OVER_H;

    if th > max_th {
        th = max_th;
        tw = th * TILE_ASPECT_W_OVER_H;
    }
    if tw > max_tw {
        tw = max_tw;
        th = tw / TILE_ASPECT_W_OVER_H;
    }
    if tw < TILE_MIN_W_PX {
        tw = TILE_MIN_W_PX.min(cw);
        th = tw / TILE_ASPECT_W_OVER_H;
        if th > ch {
            th = ch;
            tw = th * TILE_ASPECT_W_OVER_H;
        }
    }

    (tw, th)
}

fn tile_size_for_grid(avail_w: f64, avail_h: f64, cols: usize, rows: usize) -> (f64, f64) {
    let w_raw = ((avail_w - (cols - 1) as f64 * TILE_GAP_PX) / cols as f64).max(0.0);
    let h_raw = ((avail_h - (rows - 1) as f64 * TILE_GAP_PX) / rows as f64).max(0.0);
    let (base_w, _) = tile_size_16_9_in_cell(w_raw, h_raw);
    let mut tile_w = (base_w * TILE_SIZE_BOOST).min(w_raw);
    let mut tile_h = tile_w / TILE_ASPECT_W_OVER_H;
    if tile_h > h_raw {
        tile_h = h_raw;
        tile_w = tile_h * TILE_ASPECT_W_OVER_H;
    }
    (tile_w, tile_h)
}

fn single_page_layout_score(
    cols: usize,
    rows: usize,
    count: usize,
    tile_w: f64,
    tile_h: f64,
) -> f64 {
    let slots = cols * rows;
    let slot_fill = count as f64 / slots as f64;
    let row_penalty = (rows as f64).powf(ROW_COUNT_SCORE_PENALTY);

    tile_w * tile_h * slot_fill / row_penalty
}

/// Returns the usable content-box size (in logical pixels) inside `.room-page__tiles-wrap`.
pub fn tiles_wrap_content_wh(client_w: i32, client_h: i32) -> (f64, f64) {
    let w = (client_w as f64 - TILE_WRAP_PAD_X).max(0.0);
    let h = if client_h > 0 {
        (client_h as f64 - TILE_WRAP_PAD_Y).max(0.0)
    } else {
        0.0
    };
    (w, h)
}

/// Picks the `cols × rows` configuration (within `GRID_COLS × GRID_ROWS`) that maximises
/// individual tile area for `total` participants on a single page.
fn best_single_page_layout(avail_w: f64, avail_h: f64, total: usize) -> ParticipantLayout {
    let count = total.max(1);
    let aw = avail_w.max(0.0);
    let ah = avail_h.max(0.0);

    let mut best: Option<(usize, usize, f64, f64, f64)> = None;
    for rows in 1..=GRID_ROWS {
        for cols in 1..=GRID_COLS {
            if cols * rows < count {
                continue;
            }
            let (tile_w, tile_h) = tile_size_for_grid(aw, ah, cols, rows);
            let score = single_page_layout_score(cols, rows, count, tile_w, tile_h);
            let slots = cols * rows;
            match best {
                None => best = Some((cols, rows, tile_w, tile_h, score)),
                Some((bc, br, _, _, bs)) => {
                    if score > bs || ((score - bs).abs() < 0.01 && slots < bc * br) {
                        best = Some((cols, rows, tile_w, tile_h, score));
                    }
                }
            }
        }
    }

    if let Some((cols, rows, tile_w, tile_h, _)) = best {
        ParticipantLayout {
            cols,
            rows,
            tile_width: tile_w,
            tile_height: tile_h,
            per_page: TILES_PER_PAGE,
            page_count: 1,
        }
    } else {
        ParticipantLayout {
            cols: 1,
            rows: 1,
            tile_width: TILE_MIN_W_PX,
            tile_height: TILE_MIN_W_PX / TILE_ASPECT_W_OVER_H,
            per_page: TILES_PER_PAGE,
            page_count: 1,
        }
    }
}

/// Computes the paged `GRID_COLS × GRID_ROWS` layout.
/// Falls through to [`best_single_page_layout`] when all participants fit on one page,
/// so tiles are centred and as large as possible.
fn paged_grid_layout(avail_w: f64, avail_h: f64, total: usize) -> ParticipantLayout {
    if total <= TILES_PER_PAGE {
        return best_single_page_layout(avail_w, avail_h, total);
    }

    let aw = avail_w.max(0.0);
    let ah = avail_h.max(0.0);
    let (tile_w, tile_h) = tile_size_for_grid(aw, ah, GRID_COLS, GRID_ROWS);

    ParticipantLayout {
        cols: GRID_COLS,
        rows: GRID_ROWS,
        tile_width: tile_w,
        tile_height: tile_h,
        per_page: TILES_PER_PAGE,
        page_count: (total + TILES_PER_PAGE - 1) / TILES_PER_PAGE,
    }
}

/// Resolves the final layout and whether paging arrows should be shown.
/// Shrinks the available width by two arrow-slot widths when pagination is needed.
pub fn resolve_participant_layout(
    viewport_w: f64,
    viewport_h: f64,
    total: usize,
) -> (ParticipantLayout, bool) {
    let vw = viewport_w.max(0.0);
    let vh = viewport_h.max(0.0);
    let fw = vw.max(0.0);
    let fh = vh.max(0.0);

    if total == 0 {
        return (paged_grid_layout(fw, fh, 1), false);
    }

    let probe = paged_grid_layout(fw, fh, total);
    let layout = if probe.page_count <= 1 {
        paged_grid_layout(fw, fh, total)
    } else {
        let inner_w = (vw - 2.0 * ARROW_SLOT_PX).max(0.0);
        paged_grid_layout(inner_w, fh, total)
    };

    (layout, layout.page_count > 1)
}
