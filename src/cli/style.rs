//! ANSI styling primitives. View-layer helper, no business logic.
//! Conditionally emits escapes via anstyle when `enabled` is true.

use anstyle::{AnsiColor, Effects, Style as AStyle};
use std::io::IsTerminal;

#[derive(Default)]
pub(super) struct Style {
    pub(super) enabled: bool,
}

impl Style {
    pub(super) fn resolve(force_off: bool) -> Self {
        if force_off {
            return Self { enabled: false };
        }
        if std::env::var_os("NO_COLOR").is_some() {
            return Self { enabled: false };
        }
        Self { enabled: std::io::stdout().is_terminal() }
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
}
