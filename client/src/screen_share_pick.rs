//! Screen-share source enumeration and system capture. UI lives in [`crate::components::ScreenShareDialog`].

/// One selectable **display** (monitor) in the screen-share dialog — not application windows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenShareSource {
    pub id: String,
    pub label: String,
    pub width_px: u32,
    pub height_px: u32,
}

/// Load targets to render as previews in the custom picker.
///
/// TODO: Replace with real **monitor-only** enumeration (e.g. Screen Detailed API / `getScreenDetails`)
/// or whatever your pipeline uses; do not list individual windows.
/// Stub data is for layout only.
pub async fn enumerate_screen_share_sources_for_dialog() -> Vec<ScreenShareSource> {
    // Tiny yield so callers can show a loading state like a real async query.
    gloo_timers::future::TimeoutFuture::new(0).await;
    vec![
        ScreenShareSource {
            id: "stub-display-0".to_string(),
            label: "Display 1".to_string(),
            width_px: 1920,
            height_px: 1080,
        },
        ScreenShareSource {
            id: "stub-display-1".to_string(),
            label: "Display 2 (ultrawide)".to_string(),
            width_px: 3440,
            height_px: 1440,
        },
    ]
}

/// Start sharing the surface chosen in the dialog using the OS/browser capture path.
///
/// TODO: Implement with `getDisplayMedia` restricted to **full screen** capture only
/// (e.g. `displaySurface: 'monitor'` / equivalent so the browser does not offer single windows).
/// Then publish the `MediaStream` / video track to WebRTC.
pub async fn start_system_screen_share_for_stream(source_id: &str) -> Result<(), String> {
    let _ = source_id;
    gloo_timers::future::TimeoutFuture::new(0).await;
    // TODO: Call `getDisplayMedia`, then hand the `MediaStream` / video track to WebRTC.
    // Returning Ok allows the room UI to reflect "sharing" until the pipeline exists.
    Ok(())
}
