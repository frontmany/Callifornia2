//! Root shell: wires components and document-wide effects (e.g. theme class on `<html>`).

use crate::components::{MainMenu, NicknameEntry, ThemeToggle};
use crate::connector_api::logout_best_effort;
use crate::theme::Theme;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, UnwrapThrowExt};
use web_sys::{Event, HtmlElement};
use yew::prelude::*;

#[function_component]
pub fn App() -> Html {
    let theme = use_state(|| Theme::Light);
    let screen = use_state(|| AppScreen::Nickname);
    let session_id = use_state(|| Option::<String>::None);
    let handoff_complete = use_state(|| false);

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
        let session_id = session_id.clone();
        let handoff_complete = handoff_complete.clone();
        Callback::from(move |new_session_id: String| {
            session_id.set(Some(new_session_id));
            handoff_complete.set(false);
            screen.set(AppScreen::MainMenu);
        })
    };

    let on_back_to_nickname = {
        let screen = screen.clone();
        let session_id = session_id.clone();
        Callback::from(move |_| {
            session_id.set(None);
            screen.set(AppScreen::Nickname);
        })
    };

    let on_handoff_complete = {
        let handoff_complete = handoff_complete.clone();
        Callback::from(move |_| handoff_complete.set(true))
    };

    {
        let session_id_value = (*session_id).clone();
        let handoff_done = *handoff_complete;
        use_effect_with((session_id_value, handoff_done), move |(session_id, handoff_done)| {
            let listener = if let Some(session_id) = session_id.clone() {
                if !*handoff_done {
                    let listener_session_id = session_id.clone();
                    let callback = Closure::wrap(Box::new(move |_event: Event| {
                        logout_best_effort(listener_session_id.clone());
                    }) as Box<dyn FnMut(_)>);
                    let window = web_sys::window().unwrap_throw();
                    window
                        .add_event_listener_with_callback(
                            "pagehide",
                            callback.as_ref().unchecked_ref(),
                        )
                        .unwrap_throw();
                    Some((window, callback))
                } else {
                    None
                }
            } else {
                None
            };
            move || {
                if let Some((window, callback)) = listener {
                    let _ = window.remove_event_listener_with_callback(
                        "pagehide",
                        callback.as_ref().unchecked_ref(),
                    );
                    drop(callback);
                }
            }
        });
    }

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
                    session_id={(*session_id).clone().unwrap_or_default()}
                    on_handoff_complete={on_handoff_complete}
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
