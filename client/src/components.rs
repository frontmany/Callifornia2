//! Yew shells: nickname flow, main menu, theme toggle. Domain rules live in [`crate::nickname`] and [`crate::theme`].

pub mod main_menu;
pub mod nickname_entry;
pub mod theme_toggle;

pub use main_menu::MainMenu;
pub use nickname_entry::NicknameEntry;
pub use theme_toggle::ThemeToggle;
