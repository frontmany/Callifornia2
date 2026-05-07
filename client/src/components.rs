//! Yew shells: nickname flow, main menu, device-left-panel, theme toggle.
//! Domain rules live in [`crate::nickname`] and [`crate::theme`].

pub mod device_settings_left_panel;
pub mod main_menu;
pub mod nickname_entry;
pub mod theme_toggle;

pub use device_settings_left_panel::DeviceSettingsLeftPanel;
pub use main_menu::MainMenu;
pub use nickname_entry::NicknameEntry;
pub use theme_toggle::ThemeToggle;
