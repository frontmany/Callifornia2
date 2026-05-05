use crate::theme::Theme;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct ThemeToggleProps {
    pub theme: Theme,
    pub on_change: Callback<Theme>,
}

#[function_component]
pub fn ThemeToggle(props: &ThemeToggleProps) -> Html {
    let onclick = {
        let on_change = props.on_change.clone();
        let theme = props.theme;
        Callback::from(move |_| on_change.emit(theme.toggle()))
    };

    let track_class = classes!(
        "theme-toggle__track",
        match props.theme {
            Theme::Light => Some("theme-toggle__track--light"),
            Theme::Dark => Some("theme-toggle__track--dark"),
        }
    );

    html! {
        <button
            type="button"
            class="theme-toggle font-manrope"
            onclick={onclick}
            aria-label={"Switch theme"}
            aria-pressed={(matches!(props.theme, Theme::Dark)).to_string()}
            title={format!("Switch to {} theme", props.theme.next_label())}
        >
            <span class={track_class}>
                <span class="theme-toggle__thumb" />
            </span>
        </button>
    }
}
