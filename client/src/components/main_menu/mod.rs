//! Main screen after nickname: room choice and global chrome.

mod icons;

use crate::components::{DeviceSettingsLeftPanel, ThemeToggle};
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
    let is_settings_open = use_state(|| false);
    let mic_enabled = use_state(|| true);
    let camera_enabled = use_state(|| true);
    let input_level = use_state(|| 100u32);
    let output_level = use_state(|| 100u32);

    let open_settings = {
        let is_settings_open = is_settings_open.clone();
        Callback::from(move |_| is_settings_open.set(true))
    };

    let close_settings = {
        let is_settings_open = is_settings_open.clone();
        Callback::from(move |_| is_settings_open.set(false))
    };

    let toggle_mic = {
        let mic_enabled = mic_enabled.clone();
        Callback::from(move |_| mic_enabled.set(!*mic_enabled))
    };

    let toggle_camera = {
        let camera_enabled = camera_enabled.clone();
        Callback::from(move |_| camera_enabled.set(!*camera_enabled))
    };

    let on_input_level_change = {
        let input_level = input_level.clone();
        Callback::from(move |next: u32| input_level.set(next))
    };

    let on_output_level_change = {
        let output_level = output_level.clone();
        Callback::from(move |next: u32| output_level.set(next))
    };

    html! {
        <div class="main-menu font-manrope text-on-background">
            <div class="main-menu__actions main-menu__actions--floating">
                <ThemeToggle theme={props.theme} on_change={props.on_theme_change.clone()} />
                <button
                    type="button"
                    class="main-menu__icon-btn main-menu__icon-btn--settings"
                    aria-label="Settings"
                    aria-haspopup="dialog"
                    onclick={open_settings}
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

            if *is_settings_open {
                <DeviceSettingsLeftPanel
                    on_close={close_settings}
                    mic_enabled={*mic_enabled}
                    camera_enabled={*camera_enabled}
                    input_level={*input_level}
                    output_level={*output_level}
                    on_toggle_mic={toggle_mic}
                    on_toggle_camera={toggle_camera}
                    on_input_level_change={on_input_level_change}
                    on_output_level_change={on_output_level_change}
                />
            }
        </div>
    }
}
