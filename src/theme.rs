//! Opaline-backed theme loading and UI styles.

use opaline::{
    Theme, builtins,
    names::{gradients, styles, tokens},
};
use ratatui::{
    style::{Color, Modifier, Style},
    text::Line,
};

pub const DEFAULT_THEME: &str = "ayu-dark";

#[derive(Clone, Debug)]
pub struct UiTheme {
    pub name: String,
    pub display_name: String,
    pub theme: Theme,
}

impl UiTheme {
    pub fn load(name: &str) -> Self {
        let canonical = canonical_theme(name);
        let theme = builtins::load_by_name(&canonical).unwrap_or_default();
        let display_name = theme.meta.name.clone();
        Self {
            name: canonical,
            display_name,
            theme,
        }
    }

    pub fn color(&self, token: &str) -> Color {
        self.theme.color(token).into()
    }

    pub fn style(&self, name: &str) -> Style {
        self.theme.style(name).into()
    }

    pub fn fg(&self, token: &str) -> Style {
        Style::default().fg(self.color(token))
    }

    /// Canvas style: paint this first across the whole frame.
    pub fn base(&self) -> Style {
        Style::default()
            .bg(self.color(tokens::BG_BASE))
            .fg(self.color(tokens::TEXT_PRIMARY))
    }

    /// Style for secondary panes and dialogs.
    pub fn panel(&self) -> Style {
        Style::default()
            .bg(self.color(tokens::BG_PANEL))
            .fg(self.color(tokens::TEXT_PRIMARY))
    }

    pub fn text(&self) -> Style {
        self.fg(tokens::TEXT_PRIMARY)
    }

    pub fn muted(&self) -> Style {
        self.fg(tokens::TEXT_MUTED)
    }

    pub fn dim(&self) -> Style {
        self.fg(tokens::TEXT_DIM)
    }

    pub fn strong(&self) -> Style {
        self.text().add_modifier(Modifier::BOLD)
    }

    pub fn heading(&self) -> Style {
        self.fg(tokens::ACCENT_PRIMARY).add_modifier(Modifier::BOLD)
    }

    pub fn accent(&self) -> Style {
        self.fg(tokens::ACCENT_PRIMARY)
    }

    pub fn key(&self) -> Style {
        self.fg(tokens::ACCENT_SECONDARY)
            .add_modifier(Modifier::BOLD)
    }

    pub fn selected(&self) -> Style {
        self.style(styles::SELECTED)
    }

    pub fn inactive_selection(&self) -> Style {
        Style::default()
            .bg(self.color(tokens::BG_HIGHLIGHT))
            .fg(self.color(tokens::TEXT_PRIMARY))
    }

    pub fn border(&self, focused: bool) -> Style {
        if focused {
            self.style(styles::FOCUSED_BORDER)
        } else {
            self.style(styles::UNFOCUSED_BORDER)
        }
    }

    pub fn title(&self, focused: bool) -> Style {
        if focused {
            self.heading()
        } else {
            self.muted()
        }
    }

    pub fn brand_line(&self, content: &str) -> Line<'static> {
        if self.theme.has_gradient(gradients::PRIMARY) {
            self.theme.gradient_text(gradients::PRIMARY, content)
        } else {
            Line::styled(content.to_string(), self.heading())
        }
    }

    // Legacy helpers kept for the rest of the UI.

    pub fn pane_border(&self) -> Style {
        self.border(false)
    }

    pub fn pane_border_active(&self) -> Style {
        self.border(true)
    }

    pub fn pane_title(&self) -> Style {
        self.title(false)
    }

    pub fn pane_title_active(&self) -> Style {
        self.title(true)
    }

    pub fn artist(&self) -> Style {
        self.heading()
    }

    pub fn album(&self) -> Style {
        self.strong()
    }

    pub fn track(&self) -> Style {
        self.text()
    }

    pub fn list_cursor(&self) -> Style {
        self.selected()
    }

    pub fn detail_cursor(&self) -> Style {
        self.selected()
    }

    pub fn label(&self) -> Style {
        self.muted()
    }

    pub fn value(&self) -> Style {
        self.text()
    }

    pub fn mixed(&self) -> Style {
        self.muted().add_modifier(Modifier::ITALIC)
    }

    pub fn help(&self) -> Style {
        self.muted()
    }

    pub fn ok(&self) -> Style {
        self.style(styles::SUCCESS_STYLE)
    }

    pub fn error(&self) -> Style {
        self.style(styles::ERROR_STYLE)
    }

    pub fn info(&self) -> Style {
        self.style(styles::INFO_STYLE)
    }

    pub fn help_border(&self) -> Style {
        self.accent()
    }

    pub fn confirm_border(&self) -> Style {
        self.fg(tokens::WARNING)
    }

    pub fn editor_label(&self) -> Style {
        self.heading()
    }

    pub fn editor_hint(&self) -> Style {
        self.muted()
    }
}

/// Normalize a user-supplied theme name to an opaline builtin ID.
pub fn canonical_theme(name: &str) -> String {
    match name.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "" | "default" | "ayu" | "ayu dark" => DEFAULT_THEME.to_string(),
        "gruvbox" | "gruv" => "gruvbox-dark".to_string(),
        "catppuccin" => "catppuccin-mocha".to_string(),
        other => other.to_string(),
    }
}

/// Every theme shipped by opaline.
pub fn available_themes() -> Vec<(String, String)> {
    let mut themes: Vec<(String, String)> = builtins::builtin_names()
        .iter()
        .map(|(id, name)| ((*id).to_string(), (*name).to_string()))
        .collect();
    themes.sort_by_key(|a| a.1.to_ascii_lowercase());
    themes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_builtin_themes() {
        let ayu = UiTheme::load("ayu-dark");
        assert_eq!(ayu.name, "ayu-dark");
        assert!(ayu.theme.meta.name.contains("Ayu"));
        let dracula = UiTheme::load("dracula");
        assert_eq!(dracula.name, "dracula");
        assert_eq!(dracula.theme.meta.name, "Dracula");
        assert_ne!(
            ayu.color("accent.primary"),
            dracula.color("accent.primary")
        );
    }

    #[test]
    fn available_themes_are_opaline_builtins() {
        let available = available_themes();
        assert_eq!(available.len(), builtins::builtin_names().len());
        assert!(available.iter().any(|(id, _)| id == "ayu-dark"));
        assert!(available.iter().any(|(id, _)| id == "dracula"));
        assert!(!available.iter().any(|(id, _)| id == "shelf"));
    }

    #[test]
    fn aliases() {
        assert_eq!(canonical_theme("ayu dark"), "ayu-dark");
        assert_eq!(canonical_theme("ayu_dark"), "ayu-dark");
        assert_eq!(canonical_theme("gruvbox"), "gruvbox-dark");
        assert_eq!(canonical_theme("catppuccin"), "catppuccin-mocha");
    }
}
