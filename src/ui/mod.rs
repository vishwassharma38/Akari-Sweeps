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

    // Command-entry field (visible on every screen).
    render_command_entry(frame, app, vertical[3]);
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
                OpStatus::Warning => ("!", Color::Yellow),
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
    let elev = if app.elevated { "admin" } else { "not admin" };
    format!(
        "[{}|{}] exit={} · 1-5 select · / input · c clean · r repair · ? help · q/Esc quit",
        exec, elev, app.exit_code
    )
}

/// Dedicated command-entry field. `/` focuses it (shortcuts suspend
/// while typing); Enter submits through the same dispatch as number
/// keys; Esc unfocuses; Backspace edits; Ctrl+U clears.
fn render_command_entry(frame: &mut Frame, app: &App, area: Rect) {
    let (title, hint) = if app.input_focused {
        (" Command (typing — Enter runs, Esc cancels) ", "▌")
    } else {
        (" Command ( / to type ) ", "")
    };
    let body = format!("> {}{}", app.input, hint);
    let entry = Paragraph::new(Line::from(vec![
        Span::raw(body),
        Span::styled(
            if app.input_focused {
                "  [Enter] run · [Esc] cancel · [⌫] edit · [Ctrl+U] clear"
            } else {
                "  type 1-5 or press / and type a command name"
            },
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(entry, area);
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
    use crate::commands::AppCommand;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn product_label_constant() {
        assert_eq!(PRODUCT_LABEL, "Akari-Sweeps_V00.1");
    }

    fn buffer_text(width: u16, height: u16, app: &App) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect()
    }

    #[test]
    fn home_lists_five_commands_not_the_launcher() {
        let app = crate::app::App::new(AppCommand::Home);
        let text = buffer_text(100, 28, &app);
        assert!(text.contains(PRODUCT_LABEL));
        for name in [
            "akariclean",
            "akarihealth",
            "akariall",
            "akariabout",
            "akarihelp",
        ] {
            assert!(text.contains(name), "missing selectable {name}");
        }
        // `akari` as a bare selectable entry must not appear; the numbered
        // rows carry only the five public names.
        assert!(!text.contains("6  "), "must not offer six items");
    }

    #[test]
    fn missing_sprites_fall_back_without_crashing() {
        let mut app = crate::app::App::new(AppCommand::Home);
        app.frames.clear();
        app.sprite_error = Some("test: assets missing".to_string());
        let text = buffer_text(100, 28, &app);
        assert!(text.contains(PRODUCT_LABEL));
        assert!(text.contains("test: assets missing"));
    }

    #[test]
    fn tiny_terminal_degrades_gracefully() {
        let app = crate::app::App::new(AppCommand::Home);
        let text = buffer_text(40, 10, &app);
        assert!(text.contains(PRODUCT_LABEL));
    }
}
