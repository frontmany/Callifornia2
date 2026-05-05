mod handlers;
mod nickname_entry;

use nickname_entry::NicknameEntry;
use yew::prelude::*;

#[function_component]
fn App() -> Html {
    html! { <NicknameEntry /> }
}

fn main() {
    yew::Renderer::<App>::new().render();
}
