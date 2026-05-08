mod app;
mod components;
mod connector_api;
mod nickname;
mod theme;

use app::App;

fn main() {
    yew::Renderer::<App>::new().render();
}
