//! Home screen content.
use crate::commands::COMMAND_CATALOGUE;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

pub fn lines() -> Vec<Line<'static>> {
    let mut out = vec![
        Line::from(Span::styled(
            "Welcome — pick a command:",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
    ];
    for cmd in COMMAND_CATALOGUE {
        out.push(Line::from(vec![
            Span::styled(
                format!("  {:<12}", cmd.public_name()),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(format!("— {}", cmd.description())),
        ]));
    }
    out.push(Line::from(""));
    out.push(Line::from(
        "Keys: 1-6 run a command · ? help · a about · q/Esc quit.",
    ));
    out.push(Line::from(
        "Cleanup/repair start only from their explicit commands.",
    ));
    out
}
