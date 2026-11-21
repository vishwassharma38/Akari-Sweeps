//! About screen content.
use crate::app::{PRODUCT_LABEL, PRODUCT_VERSION};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

pub fn lines() -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            PRODUCT_LABEL.to_string(),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(format!(
            "Version V{PRODUCT_VERSION} — Windows maintenance TUI."
        )),
        Line::from(""),
        Line::from("Windows cleanup and system-image repair tools in an interactive"),
        Line::from("terminal interface featuring Akari animations."),
        Line::from(""),
        Line::from("Repository: vishwassharma38/Akari-Sweeps (MIT)."),
        Line::from("Source and project information: github.com/vishwassharma38/Akari-Sweeps."),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_uses_canonical_version_and_repository_information() {
        let rendered = lines()
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains(&format!("Version V{PRODUCT_VERSION}")));
        assert_eq!(PRODUCT_VERSION, env!("CARGO_PKG_VERSION"));
        assert!(rendered.contains("vishwassharma38/Akari-Sweeps"));
        assert!(!rendered.contains("Behavioral spec"));
    }
}
