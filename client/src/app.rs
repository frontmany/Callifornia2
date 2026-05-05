//! Root shell: wires components and document-wide effects (e.g. theme class on `<html>`).

use crate::components::{NicknameEntry, ThemeToggle};
use crate::theme::Theme;
use wasm_bindgen::{JsCast, UnwrapThrowExt};
use web_sys::HtmlElement;
use yew::prelude::*;

#[function_component]
pub fn App() -> Html {
    let theme = use_state(|| Theme::Light);

    {
        let theme = *theme;
        use_effect_with(theme, move |t| {
            let window = web_sys::window().unwrap_throw();
            let document = window.document().unwrap_throw();
            if let Some(root) = document.document_element() {
                let html: HtmlElement = root.dyn_into().unwrap_throw();
                html.set_class_name(t.html_class());
            }
        });
    }

    let on_theme = {
        let theme = theme.clone();
        Callback::from(move |next: Theme| theme.set(next))
    };

    html! {
        <>
            <ThemeToggle theme={*theme} on_change={on_theme} />
            <NicknameEntry />
        </>
    }
}
