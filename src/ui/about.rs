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
            "Version {PRODUCT_VERSION} — Windows maintenance TUI."
        )),
        Line::from(""),
        Line::from("Reproduces the Akari-Sweeps.bat cleanup + system-image repair"),
        Line::from("workflow in Rust, with Akari's idle reading and sweep animations."),
        Line::from(""),
        Line::from("Repository: vishwassharma38/Akari-Sweeps (MIT)."),
        Line::from("Behavioral spec: Akari-Sweeps.bat in the project root."),
        Line::from(""),
        Line::from("Cleanup never runs on startup unless akariclean/akariall requested it."),
    ]
}
