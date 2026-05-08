//! Nickname flow logic: validation and connector auth call.

use crate::connector_api::AuthResponse;

pub const MIN_NICKNAME_LEN: usize = 3;
pub const MAX_NICKNAME_LEN: usize = 24;

const ERR_CHARS: &str = "Use only English letters, numbers, and underscores";
const ERR_WHITESPACE: &str = "Spaces and line breaks are not allowed";

pub async fn submit_nickname(nickname: &str) -> Result<AuthResponse, String> {
    crate::connector_api::auth(nickname).await
}

fn ascii_nickname_chars_only(value: &str) -> bool {
    value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn validate_no_whitespace(value: &str) -> Result<(), &'static str> {
    if value.chars().any(char::is_whitespace) {
        Err(ERR_WHITESPACE)
    } else {
        Ok(())
    }
}

fn validate_charset(value: &str) -> Result<(), &'static str> {
    validate_no_whitespace(value)?;
    if ascii_nickname_chars_only(value) {
        Ok(())
    } else {
        Err(ERR_CHARS)
    }
}

fn validate_shape_when_nonempty(value: &str) -> Result<(), &'static str> {
    if value.is_empty() {
        return Err("Nickname is required");
    }

    if value.len() < MIN_NICKNAME_LEN {
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

pub fn validate_nickname(value: &str) -> Result<(), &'static str> {
    validate_charset(value)?;
    validate_shape_when_nonempty(value)
}

pub fn validate_nickname_live(value: &str, submit_attempted: bool) -> Result<(), &'static str> {
    validate_charset(value)?;
    if submit_attempted {
        validate_shape_when_nonempty(value)?;
    }
    Ok(())
}
