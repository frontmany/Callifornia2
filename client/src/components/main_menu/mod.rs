//! Main screen after nickname: room choice and global chrome (theme toggle, settings stub).

mod icons;

use crate::components::ThemeToggle;
use crate::theme::Theme;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct MainMenuProps {
    pub theme: Theme,
    pub on_theme_change: Callback<Theme>,
    pub on_back: Callback<()>,
}

#[function_component]
pub fn MainMenu(props: &MainMenuProps) -> Html {
    html! {
        <div class="main-menu font-manrope text-on-background">
            <div class="main-menu__actions main-menu__actions--floating">
                <ThemeToggle theme={props.theme} on_change={props.on_theme_change.clone()} />
                <button
                    type="button"
                    class="main-menu__icon-btn main-menu__icon-btn--settings"
                    aria-label="Settings"
                >
                    { icons::settings_gear_icon() }
                </button>
            </div>

            <main class="main-menu__content">
                <header class="main-menu__heading">
                    <h1 class="h1 main-menu__title">{ "Get connected" }</h1>
                    <p class="body-lg main-menu__subtitle">{ "Start a new space or drop into an existing one." }</p>
                </header>

                <section class="main-menu__cards">
                    <button type="button" class="main-menu__card">
                        <span class="main-menu__card-icon main-menu__card-icon--primary" aria-hidden="true">
                            { icons::create_plus_icon() }
                        </span>
                        <span class="main-menu__card-title h2">{ "Create Room" }</span>
                        <span class="main-menu__card-text body-md">{ "Start a fresh session and invite your team." }</span>
                    </button>

                    <button type="button" class="main-menu__card">
                        <span class="main-menu__card-icon main-menu__card-icon--secondary" aria-hidden="true">
                            { icons::join_enter_icon() }
                        </span>
                        <span class="main-menu__card-title h2">{ "Join Room" }</span>
                        <span class="main-menu__card-text body-md">{ "Enter a link or code to hop right in." }</span>
                    </button>
                </section>

                <button
                    type="button"
                    class="main-menu__back body-md"
                    onclick={props.on_back.reform(|_| ())}
                >
                    <span class="main-menu__back-arrow" aria-hidden="true">
                        { icons::back_arrow_icon() }
                    </span>
                    <span>{ "Go Back" }</span>
                </button>
            </main>
        </div>
    }
}
