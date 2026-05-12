//! Participant tile: optional video (`MediaStream`) plus label overlay.

use super::tiles::{MAX_NAME_CHARS, truncate_str};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use web_sys::HtmlVideoElement;
use yew::prelude::*;

#[derive(Properties, PartialEq, Clone)]
pub struct ParticipantTileProps {
    pub name: String,
    /// When set, must be a [`web_sys::MediaStream`] as [`JsValue`] (pass `stream.into()`).
    #[prop_or_default]
    pub media: Option<JsValue>,
    pub width_px: f64,
    pub height_px: f64,
    #[prop_or_default]
    pub rail: bool,
    /// Local preview: `true` avoids echo; remote participants use `false` so audio plays.
    /// Громкость воспроизведения задаётся глобально в [`super::Room`] для всех `<video>` сразу.
    pub mute_audio: bool,
}

/// Wraps a [`MediaStream`](web_sys::MediaStream) for [`RoomProps::participant_media`](super::RoomProps::participant_media).
#[allow(dead_code)]
pub fn media_stream_js(stream: web_sys::MediaStream) -> JsValue {
    stream.into()
}

fn media_stream_has_video(media: &Option<JsValue>) -> bool {
    let Some(js) = media.as_ref() else {
        return false;
    };
    let Ok(stream) = js.clone().dyn_into::<web_sys::MediaStream>() else {
        return false;
    };
    stream.get_video_tracks().length() > 0
}

#[function_component]
pub fn ParticipantTile(props: &ParticipantTileProps) -> Html {
    let video_ref = use_node_ref();
    let has_video = media_stream_has_video(&props.media);

    {
        let video_ref = video_ref.clone();
        let media = props.media.clone();
        let mute_audio = props.mute_audio;
        use_effect_with((media.clone(), mute_audio), move |(media, mute_audio)| {
            let has_video = media_stream_has_video(media);
            if let Some(video_el) = video_ref.cast::<HtmlVideoElement>() {
                let _ = video_el.set_muted(*mute_audio);
                if !has_video {
                    let _ = video_el.set_src_object(None);
                } else if let Some(ms) = media
                    .as_ref()
                    .and_then(|j| j.clone().dyn_into::<web_sys::MediaStream>().ok())
                {
                    let _ = video_el.set_src_object(Some(&ms));
                }
            }
            || ()
        });
    }

    let label = truncate_str(&props.name, MAX_NAME_CHARS);
    let style = format!(
        "width:{w:.1}px;height:{h:.1}px;flex:0 0 auto;",
        w = props.width_px,
        h = props.height_px
    );

    html! {
        <div
            class={classes!(
                "room-page__tile",
                props.rail.then_some("room-page__tile--rail"),
                has_video.then_some("room-page__tile--has-video"),
            )}
            style={style}
        >
            if has_video {
                <video
                    ref={video_ref}
                    class="room-page__tile-video"
                    autoplay=true
                    playsinline=true
                    muted={props.mute_audio}
                    aria-label={format!("{} camera", props.name)}
                />
            }
            <span
                class="room-page__tile-name"
                title={props.name.clone()}
            >
                { label }
            </span>
        </div>
    }
}
