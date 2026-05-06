//! Root shell: wires components and document-wide effects (e.g. theme class on `<html>`).

use crate::components::{MainMenu, NicknameEntry, ThemeToggle};
use crate::theme::Theme;
use wasm_bindgen::{JsCast, UnwrapThrowExt};
use web_sys::HtmlElement;
use yew::prelude::*;

#[function_component]
pub fn App() -> Html {
    let theme = use_state(|| Theme::Light);
    let screen = use_state(|| AppScreen::Nickname);

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

    let on_nickname_success = {
        let screen = screen.clone();
        Callback::from(move |_nickname: String| screen.set(AppScreen::MainMenu))
    };

    let on_back_to_nickname = {
        let screen = screen.clone();
        Callback::from(move |_| screen.set(AppScreen::Nickname))
    };

    html! {
        match *screen {
            AppScreen::Nickname => html! {
                <>
                    <ThemeToggle theme={*theme} on_change={on_theme.clone()} />
                    <NicknameEntry on_success={on_nickname_success} />
                </>
            },
            AppScreen::MainMenu => html! {
                <MainMenu
                    theme={*theme}
                    on_theme_change={on_theme}
                    on_back={on_back_to_nickname}
                />
            },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppScreen {
    Nickname,
    MainMenu,
}
