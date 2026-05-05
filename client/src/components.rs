//! Yew UI: markup and local interaction state only. Rules live in [`crate::nickname`] and [`crate::theme`].

pub mod nickname_entry;
pub mod theme_toggle;

pub use nickname_entry::NicknameEntry;
pub use theme_toggle::ThemeToggle;
