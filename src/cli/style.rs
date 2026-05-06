//! ANSI styling primitives. View-layer helper, no business logic.
//! Conditionally emits escapes via anstyle when `enabled` is true.

use anstyle::{AnsiColor, Effects, Style as AStyle};
use std::io::IsTerminal;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Default)]
pub(super) struct Style {
    pub(super) enabled: bool,
}

impl Style {
    /// Resolve a `--color` choice into an enabled/disabled style.
    /// `is_tty` and `no_color_env` are injected so this is pure / testable.
    pub(super) fn from_choice(choice: ColorChoice, is_tty: bool, no_color_env: bool) -> Self {
        let enabled = match choice {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => is_tty && !no_color_env,
        };
        Self { enabled }
    }

    /// Convenience for run_ls: read live env + tty.
    pub(super) fn resolve(choice: ColorChoice) -> Self {
        let is_tty = std::io::stdout().is_terminal();
        let no_color_env = std::env::var_os("NO_COLOR").is_some();
        Self::from_choice(choice, is_tty, no_color_env)
    }

    fn paint(&self, style: AStyle, s: &str) -> String {
        if self.enabled {
            format!("{style}{s}{style:#}")
        } else {
            s.to_string()
        }
    }

    pub(super) fn bold(&self, s: &str) -> String {
        self.paint(AStyle::new().effects(Effects::BOLD), s)
    }
    pub(super) fn dim(&self, s: &str) -> String {
        self.paint(AStyle::new().effects(Effects::DIMMED), s)
    }
    pub(super) fn green(&self, s: &str) -> String {
        self.paint(AStyle::new().fg_color(Some(AnsiColor::Green.into())), s)
    }
    pub(super) fn amber(&self, s: &str) -> String {
        self.paint(AStyle::new().fg_color(Some(AnsiColor::Yellow.into())), s)
    }
    pub(super) fn cyan(&self, s: &str) -> String {
        self.paint(AStyle::new().fg_color(Some(AnsiColor::Cyan.into())), s)
    }
    pub(super) fn port(&self, s: &str) -> String {
        self.paint(
            AStyle::new()
                .fg_color(Some(AnsiColor::BrightBlue.into()))
                .effects(Effects::BOLD),
            s,
        )
    }
    pub(super) fn url(&self, s: &str) -> String {
        self.paint(
            AStyle::new()
                .fg_color(Some(AnsiColor::Blue.into()))
                .effects(Effects::UNDERLINE),
            s,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_disabled_emits_no_escapes() {
        let s = Style { enabled: false };
        assert_eq!(s.green("ok"), "ok");
        assert_eq!(s.bold("hi"), "hi");
    }

    #[test]
    fn style_enabled_wraps_with_escapes() {
        let s = Style { enabled: true };
        let out = s.green("ok");
        assert!(out.starts_with("\x1b[32m"));
        assert!(out.ends_with("\x1b[0m"));
    }

    // ─── ColorChoice / from_choice ─────────────────────────────────

    #[test]
    fn color_choice_always_enables_even_without_tty() {
        let s = Style::from_choice(ColorChoice::Always, /*is_tty=*/ false, /*no_color_env=*/ false);
        assert!(s.enabled);
    }

    #[test]
    fn color_choice_never_disables_even_with_tty() {
        let s = Style::from_choice(ColorChoice::Never, /*is_tty=*/ true, /*no_color_env=*/ false);
        assert!(!s.enabled);
    }

    #[test]
    fn color_choice_auto_follows_tty() {
        let on = Style::from_choice(ColorChoice::Auto, /*is_tty=*/ true, /*no_color_env=*/ false);
        let off = Style::from_choice(ColorChoice::Auto, /*is_tty=*/ false, /*no_color_env=*/ false);
        assert!(on.enabled);
        assert!(!off.enabled);
    }

    #[test]
    fn color_choice_auto_respects_no_color_env() {
        let s = Style::from_choice(ColorChoice::Auto, /*is_tty=*/ true, /*no_color_env=*/ true);
        assert!(!s.enabled);
    }

    #[test]
    fn color_choice_always_ignores_no_color_env() {
        // --color=always is an explicit user request; NO_COLOR env shouldn't override.
        let s = Style::from_choice(ColorChoice::Always, /*is_tty=*/ true, /*no_color_env=*/ true);
        assert!(s.enabled);
    }
}
