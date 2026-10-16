//! Home screen content: the five selectable commands.
//!
//! `akari` launches this interface and is therefore not a selectable
//! item inside it (see `SELECTABLE_COMMANDS`).
use crate::commands::SELECTABLE_COMMANDS;
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
    for (i, cmd) in SELECTABLE_COMMANDS.iter().enumerate() {
        out.push(Line::from(vec![
            Span::styled(
                format!("  {}  {:<12}", i + 1, cmd.public_name()),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(format!("— {}", cmd.description())),
        ]));
    }
    out.push(Line::from(""));
    out.push(Line::from(
        "Keys: 1-5 select · / command entry · -/back home · ? help · a about · q/Esc quit.",
    ));
    out.push(Line::from(
        "Cleanup/repair start only from their explicit commands.",
    ));
    out.push(Line::from(
        "After an operation: type back, akariback, or press - to return here. 1 never means exit.",
    ));
    out
}
