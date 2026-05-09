mod app;
mod components;
mod connector_api;
mod nickname;
mod screen_share_pick;
mod theme;

use app::App;

fn main() {
    yew::Renderer::<App>::new().render();
}
