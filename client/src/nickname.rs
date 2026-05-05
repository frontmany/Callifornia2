//! Nickname flow logic: validation and side effects (no Yew types).

pub mod actions;
pub mod validation;

pub use actions::submit_nickname;
pub use validation::{validate_nickname, validate_nickname_live};
