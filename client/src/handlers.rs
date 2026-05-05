pub const MAX_NICKNAME_LEN: usize = 24;

pub fn validate_nickname(value: &str) -> Result<(), &'static str> {
    if value.is_empty() {
        return Err("Nickname is required");
    }

    if !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("Use only letters, numbers, and underscores");
    }

    if value.len() < 3 {
        return Err("Use at least 3 characters");
    }

    if value.len() > MAX_NICKNAME_LEN {
        return Err("Use up to 24 characters");
    }

    if value.chars().all(|c| c == '_') {
        return Err("Nickname cannot contain only underscores");
    }

    Ok(())
}

/// Validation while typing: length / empty-field messages appear only after the user tried to submit.
pub fn validate_nickname_live(value: &str, submit_attempted: bool) -> Result<(), &'static str> {
    if !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("Use only english letters, numbers, and underscores");
    }

    if !submit_attempted {
        return Ok(());
    }

    if value.is_empty() {
        return Err("Nickname is required");
    }

    if value.len() < 3 {
        return Err("Use at least 3 characters");
    }

    if value.len() > MAX_NICKNAME_LEN {
        return Err("Use up to 24 characters");
    }

    if value.chars().all(|c| c == '_') {
        return Err("Nickname cannot contain only underscores");
    }

    Ok(())
}

pub fn handle_nickname_change(raw: &str) -> String {
    raw.to_owned()
}

pub fn handle_continue_click(_nickname: &str) {
    // TODO: Replace with WS request to connector when backend contract is ready.
}
