//! Light / dark theme identifier applied on `<html>` (`light` / `dark` classes).

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub fn html_class(self) -> &'static str {
        match self {
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::Light,
        }
    }

    pub fn next_label(self) -> &'static str {
        match self {
            Theme::Light => "Dark",
            Theme::Dark => "Light",
        }
    }
}
