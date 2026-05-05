use crate::handlers;
use web_sys::HtmlInputElement;
use yew::prelude::*;

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
            let sanitized = handlers::handle_nickname_change(&input.value());
            let validation = handlers::validate_nickname_live(&sanitized, *attempted_submit)
                .err()
                .map(str::to_string);

            nickname.set(sanitized);
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

            match handlers::validate_nickname(&value) {
                Ok(()) => {
                    validation_error.set(None);
                    handlers::handle_continue_click(&value);
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
                    <svg
                        class="nickname-entry__avatar-icon"
                        xmlns="http://www.w3.org/2000/svg"
                        viewBox="0 0 24 24"
                        width="34"
                        height="34"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.75"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                    >
                        <path d="M12 12c2.21 0 4-1.79 4-4s-1.79-4-4-4-4 1.79-4 4 1.79 4 4 4Z" />
                        <path d="M4 21c1.5-4.5 6-6.5 8-6.5s6.5 2 8 6.5" />
                    </svg>
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
                    <p class="nickname-entry__hint label-md">{ "english letters, numbers, underscores (3-24 chars)" }</p>

                    if let Some(message) = &*validation_error {
                        <p class="nickname-entry__error label-md" role="alert">{ message }</p>
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
