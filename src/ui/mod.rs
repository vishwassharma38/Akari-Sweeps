//! Ratatui layout: header (top-left identity), main/output panel,
//! right-side Akari animation panel, status bar. Adapts to small terminals.

pub mod about;
pub mod help;
pub mod home;
pub mod maintenance;
pub mod widgets;

use crate::app::{App, PRODUCT_LABEL, Screen};
use crate::system::OpStatus;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

fn header_style() -> Style {
    Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::BOLD)
}

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 60 || area.height < 18 {
        render_compact(frame, app, area);
        return;
    }

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(area);

    // Header: exact product label, top-left, visually distinct.
    let header = Paragraph::new(vec![
        Line::from(vec![Span::styled(PRODUCT_LABEL, header_style())]),
        Line::from(Span::styled(
            screen_subtitle(app),
            Style::default().fg(Color::DarkGray),
        )),
    ])
    .block(Block::default().borders(Borders::ALL).title(" Akari "));
    frame.render_widget(header, vertical[0]);

    // Body: main (2/3) + animation (1/3), visually separate.
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(66), Constraint::Percentage(34)])
        .split(vertical[1]);
    render_main(frame, app, body[0]);
    render_animation_panel(frame, app, body[1]);

    // Status bar.
    let status = Paragraph::new(status_text(app))
        .block(Block::default().borders(Borders::ALL).title(" Status "));
    frame.render_widget(status, vertical[2]);
}

fn screen_subtitle(app: &App) -> String {
    match app.screen {
        Screen::Home => format!("home — {} · ? help · q quit", app.command.public_name()),
        Screen::Maintenance => format!(
            "maintenance — {} · {} · q quit",
            app.command.public_name(),
            maintenance::progress_line(app)
        ),
        Screen::About => "about — q back".to_string(),
        Screen::Help => "help — q back".to_string(),
    }
}

fn render_main(frame: &mut Frame, app: &App, area: Rect) {
    match app.screen {
        Screen::Home => {
            let p = Paragraph::new(home::lines())
                .block(Block::default().borders(Borders::ALL).title(" Commands "))
                .wrap(Wrap { trim: false });
            frame.render_widget(p, area);
        }
        Screen::About => {
            let p = Paragraph::new(about::lines())
                .block(Block::default().borders(Borders::ALL).title(" About "))
                .wrap(Wrap { trim: false });
            frame.render_widget(p, area);
        }
        Screen::Help => {
            let p = Paragraph::new(help::lines())
                .block(Block::default().borders(Borders::ALL).title(" Help "))
                .wrap(Wrap { trim: false });
            frame.render_widget(p, area);
        }
        Screen::Maintenance => render_maintenance(frame, app, area),
    }
}

fn render_maintenance(frame: &mut Frame, app: &App, area: Rect) {
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(6)])
        .split(area);

    // Operation list with pending/running/ok/failed/skipped distinction.
    let items: Vec<ListItem> = app
        .operations
        .iter()
        .map(|op| {
            let (mark, color) = match op.status {
                OpStatus::Pending => ("○", Color::DarkGray),
                OpStatus::Running => ("◐", Color::Yellow),
                OpStatus::Success => ("✓", Color::Green),
                OpStatus::Failed => ("✗", Color::Red),
                OpStatus::Skipped => ("–", Color::DarkGray),
            };
            let line = Line::from(vec![
                Span::styled(format!("{mark} "), Style::default().fg(color)),
                Span::raw(format!("{} — {}", op.label, op.target)),
            ]);
            ListItem::new(line)
        })
        .collect();
    let list_title = format!(" Operations — {} ", maintenance::progress_line(app));
    let list = List::new(if items.is_empty() {
        vec![ListItem::new("No operations yet — starting…")]
    } else {
        items
    })
    .block(Block::default().borders(Borders::ALL).title(list_title));
    frame.render_widget(list, inner[0]);

    // Live log tail + summary.
    let mut lines: Vec<Line> = app
        .log
        .iter()
        .rev()
        .take((inner[1].height as usize).saturating_sub(2).max(1))
        .rev()
        .map(|l| Line::from(l.clone()))
        .collect();
    lines.extend(maintenance::summary_lines(app));
    let log = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Output "))
        .wrap(Wrap { trim: false });
    frame.render_widget(log, inner[1]);
}

fn render_animation_panel(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" Akari ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if app.frames.is_empty() {
        let msg = Paragraph::new(vec![
            Line::from("…sweeping…"),
            Line::from(""),
            Line::from(
                app.sprite_error
                    .clone()
                    .unwrap_or_else(|| "sprite assets unavailable".to_string()),
            ),
        ])
        .wrap(Wrap { trim: false });
        frame.render_widget(msg, inner);
        return;
    }
    let idx = app
        .animator
        .current_frame()
        .min(app.frames.len().saturating_sub(1));
    let lines = widgets::render_sprite_halfblock(
        &app.frames[idx],
        inner.width as usize,
        inner.height as usize,
    );
    // Caption: honest about fallback rendering + current activity.
    let mode = match app.animator.mode() {
        crate::akari::animation::AnimationMode::Sweeping => "sweeping…",
        crate::akari::animation::AnimationMode::Settled => "done",
        _ => "idle",
    };
    let mut all = lines;
    all.push(Line::from(Span::styled(
        format!("{mode} · terminal approx. (256×320 source)"),
        Style::default().fg(Color::DarkGray),
    )));
    // Trim to fit without overflow.
    let max = inner.height as usize;
    if all.len() > max {
        all.truncate(max);
    }
    frame.render_widget(Paragraph::new(all), inner);
}

fn status_text(app: &App) -> String {
    let exec = match app.execution {
        crate::app::ExecutionState::Idle => "idle",
        crate::app::ExecutionState::Running => "running",
        crate::app::ExecutionState::Done => "done",
        crate::app::ExecutionState::Error => "error",
    };
    format!(
        "[{}] exit={} · 1-6 commands · c clean · r repair · ? help · q/Esc quit",
        exec, app.exit_code
    )
}

fn render_compact(frame: &mut Frame, app: &App, area: Rect) {
    // Graceful degradation: single column, no overlap, no crash.
    let text = vec![
        Line::from(Span::styled(
            PRODUCT_LABEL,
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "{} · enlarge terminal for full TUI",
            status_text(app)
        )),
        Line::from(""),
        Line::from(match app.screen {
            Screen::Maintenance => maintenance::progress_line(app),
            _ => "press q to quit".to_string(),
        }),
    ];
    frame.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_label_constant() {
        assert_eq!(PRODUCT_LABEL, "Akari-Sweeps_V00.1");
    }
}
