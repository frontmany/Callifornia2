use crate::nickname::{submit_nickname, validate_nickname, validate_nickname_live};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use std::sync::LazyLock;
use web_sys::HtmlInputElement;
use yew::prelude::*;

/// Logo embedded so it still loads regardless of asset URL / Trunk copies.
static LOGO_PNG_DATA_URL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "data:image/png;base64,{}",
        B64.encode(include_bytes!("../../icons/logo.png"))
    )
});

#[function_component]
pub fn NicknameEntry() -> Html {
    let nickname = use_state(String::new);
    let validation_error = use_state(|| Option::<String>::None);
    let attempted_submit = use_state(|| false);

    let on_input = {
        let nickname = nickname.clone();
        let validation_error = validation_error.clone();
        let attempted_submit = attempted_submit.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlInputElement = event.target_unchecked_into();
            let value = input.value();
            let validation = validate_nickname_live(&value, *attempted_submit)
                .err()
                .map(str::to_string);

            nickname.set(value);
            validation_error.set(validation);
        })
    };

    let on_submit = {
        let nickname = nickname.clone();
        let validation_error = validation_error.clone();
        let attempted_submit = attempted_submit.clone();
        Callback::from(move |event: SubmitEvent| {
            event.prevent_default();
            let value = (*nickname).clone();

            attempted_submit.set(true);

            match validate_nickname(&value) {
                Ok(()) => {
                    validation_error.set(None);
                    submit_nickname(&value);
                }
                Err(message) => validation_error.set(Some(message.to_owned())),
            }
        })
    };

    let has_error = validation_error.is_some();
    let mut input_classes = classes!(
        "nickname-entry__input",
        "font-manrope",
        "body-lg",
        "text-on-surface"
    );
    if has_error {
        input_classes.push("nickname-entry__input--error");
    }

    html! {
        <main class="nickname-entry font-manrope text-on-background">
            <section class="nickname-entry__card" aria-label="Nickname entry card">
                <div class="nickname-entry__avatar" aria-hidden="true">
                    <img
                        class="nickname-entry__avatar-logo"
                        src={LOGO_PNG_DATA_URL.as_str()}
                        width="52"
                        height="52"
                        alt=""
                    />
                </div>

                <h1 class="nickname-entry__title h1">{ "What should we call you?" }</h1>
                <p class="nickname-entry__subtitle body-md">{ "Enter your nickname to start" }</p>

                <form class="nickname-entry__form" onsubmit={on_submit}>
                    <input
                        id="nickname-input"
                        class={input_classes}
                        type="text"
                        value={(*nickname).clone()}
                        oninput={on_input}
                        placeholder="e.g. Maverick"
                        autocomplete="nickname"
                    />
                    <p class="nickname-entry__hint body-md">{ "English letters, numbers, underscores, no spaces (3–24 chars)" }</p>

                    if let Some(message) = &*validation_error {
                        <p class="nickname-entry__error body-md" role="alert">{ message }</p>
                    }

                    <button class="nickname-entry__button" type="submit">
                        <span class="nickname-entry__button-label">{ "Continue" }</span>
                        <span class="nickname-entry__button-arrow" aria-hidden="true">
                            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="26" height="26" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
                                <path d="M5 12h14" />
                                <path d="M13 6l6 6-6 6" />
                            </svg>
                        </span>
                    </button>
                </form>
            </section>
        </main>
    }
}
