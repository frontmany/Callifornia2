mod app;
mod components;
mod nickname;
mod theme;

use app::App;

fn main() {
    yew::Renderer::<App>::new().render();
}
