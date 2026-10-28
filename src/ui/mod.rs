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
        Screen::Home => format!("home — {} · q quit", app.command.public_name()),
        Screen::Maintenance => format!(
            "maintenance — {} · {} · back/- home · q quit",
            app.command.public_name(),
            maintenance::progress_line(app)
        ),
        Screen::About => "about — back/- home · q quit".to_string(),
        Screen::Help => "help — back/- home · q quit".to_string(),
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
    let frames = app.frames();
    if frames.is_empty() {
        let activity = match app.animator.set() {
            crate::akari::animation::AnimationSet::Idle => "…reading…",
            crate::akari::animation::AnimationSet::Sweep => "…sweeping…",
            crate::akari::animation::AnimationSet::Health => "…repairing…",
            crate::akari::animation::AnimationSet::About => "…investigating…",
        };
        let msg = Paragraph::new(vec![
            Line::from(activity),
            Line::from(""),
            Line::from(
                app.sprite_error()
                    .map(str::to_string)
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
        .min(frames.len().saturating_sub(1));
    let shown = &frames[idx];
    // Leave the last row for the activity/source caption.
    let sprite_height = inner.height.saturating_sub(1);
    let lines =
        widgets::render_sprite_halfblock(shown, inner.width as usize, sprite_height as usize);
    // Caption: honest about fallback rendering + current activity.
    let mode = match app.animator.mode() {
        crate::akari::animation::AnimationMode::Sweeping => match app.animator.set() {
            crate::akari::animation::AnimationSet::Health => "repairing…",
            _ => "sweeping…",
        },
        crate::akari::animation::AnimationMode::Settled => "done",
        crate::akari::animation::AnimationMode::Ambient => match app.animator.set() {
            crate::akari::animation::AnimationSet::Idle => "reading",
            crate::akari::animation::AnimationSet::Sweep => "idle",
            crate::akari::animation::AnimationSet::Health => "health",
            crate::akari::animation::AnimationSet::About => "investigating",
        },
    };
    let mut all = lines;
    all.push(Line::from(Span::styled(
        format!(
            "{mode} · terminal approx. ({}×{} source)",
            shown.width, shown.height
        ),
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
    // During maintenance operations the Status panel is the accurate,
    // continuously updated progress bar (spec: `Status: [====----] 76.4%`).
    // While work is actively running it shows only the progress display
    // and its directly associated execution/elevation indicators — the
    // idle keyboard hints stay hidden until the run settles. Once the
    // run is Done/Error the navigation hints return. Elsewhere the
    // status line remains the idle/exec-state line. Error details stay
    // in the operations list + output log; the bar never hides failures.
    if app.screen == Screen::Maintenance
        && matches!(
            app.execution,
            crate::app::ExecutionState::Running
                | crate::app::ExecutionState::Done
                | crate::app::ExecutionState::Error
        )
    {
        let bar = maintenance::progress_bar_line(app);
        let exec = match app.execution {
            crate::app::ExecutionState::Idle => "idle",
            crate::app::ExecutionState::Running => "running",
            crate::app::ExecutionState::Done => "done",
            crate::app::ExecutionState::Error => "error",
        };
        let elev = if app.elevated { "admin" } else { "not admin" };
        if app.execution == crate::app::ExecutionState::Running {
            return format!("{bar} · [{exec}|{elev}]");
        }
        return format!("{bar} · [{exec}|{elev}] · -/back home · q quit");
    }
    let exec = match app.execution {
        crate::app::ExecutionState::Idle => "idle",
        crate::app::ExecutionState::Running => "running",
        crate::app::ExecutionState::Done => "done",
        crate::app::ExecutionState::Error => "error",
    };
    let elev = if app.elevated { "admin" } else { "not admin" };
    format!(
        "[{}|{}] · 1-5 select · / input · -/back home · q/Esc quit",
        exec, elev
    )
}

/// Human-readable placeholder for the command entry, shown only when the
/// input buffer is empty. Never inserted into `app.input` itself.
pub fn entry_placeholder(focused: bool) -> &'static str {
    if focused {
        "  [Enter] run · [Esc] cancel · [⌫] edit · [Ctrl+U] clear"
    } else {
        "  type 1-5 or press / and type a command name"
    }
}

/// Whether the placeholder should be visible for the given buffer.
/// Conventional text-field behavior: visible only when empty.
pub fn placeholder_visible(input: &str) -> bool {
    input.is_empty()
}

/// Dedicated command-entry field. `/` focuses it (shortcuts suspend
/// while typing); Enter submits through the same dispatch as number
/// keys; Esc unfocuses; Backspace edits; Ctrl+U clears.
///
/// Placeholder behavior is conventional: the instructional hint is a
/// separate visual span shown only when `app.input` is empty. Typing
/// the first character hides it immediately (no pushing/overlap);
/// clearing restores it. The hint is never inserted into the buffer.
fn render_command_entry(frame: &mut Frame, app: &App, area: Rect) {
    let (title, cursor) = if app.input_focused {
        (" Command (typing — Enter runs, Esc cancels) ", "▌")
    } else {
        (" Command ( / to type ) ", "")
    };
    let body = format!("> {}{}", app.input, cursor);
    let mut spans = vec![Span::raw(body)];
    if placeholder_visible(&app.input) {
        spans.push(Span::styled(
            entry_placeholder(app.input_focused),
            Style::default().fg(Color::DarkGray),
        ));
    }
    let entry = Paragraph::new(Line::from(spans))
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
        let set = app.animator.set();
        app.sprites.get_mut(set).frames.clear();
        app.sprites.get_mut(set).error = Some("test: assets missing".to_string());
        let text = buffer_text(100, 28, &app);
        assert!(text.contains(PRODUCT_LABEL));
        assert!(text.contains("test: assets missing"));
    }

    #[test]
    fn idle_panel_reports_real_source_dimensions() {
        use crate::akari::sprites::SpriteFrame;

        let mut app = crate::app::App::new(AppCommand::Home);
        let set = app.animator.set();
        let assets = app.sprites.get_mut(set);
        assets.error = None;
        assets.frames = vec![SpriteFrame {
            width: 2,
            height: 4,
            rgba: vec![255, 0, 0, 255].repeat(8),
        }];
        let text = buffer_text(160, 40, &app);
        assert!(text.contains("reading"), "idle caption must be reading");
        assert!(text.contains("2×4 source"), "caption must use frame size");
        assert!(
            !text.contains("256×320"),
            "idle caption must not claim sweep dimensions"
        );
    }

    #[test]
    fn about_panel_reports_about_caption_and_source_dimensions() {
        use crate::akari::animation::AnimationSet;
        use crate::akari::sprites::SpriteFrame;

        let mut app = crate::app::App::new(AppCommand::About);
        assert_eq!(app.animator.set(), AnimationSet::About);
        let assets = app.sprites.get_mut(AnimationSet::About);
        assets.error = None;
        assets.frames = vec![SpriteFrame {
            width: 2,
            height: 4,
            rgba: vec![0, 255, 255, 255].repeat(8),
        }];
        let text = buffer_text(160, 40, &app);
        assert!(
            text.contains("investigating"),
            "about caption must report about activity"
        );
        assert!(text.contains("2×4 source"), "caption must use frame size");
        assert!(
            !text.contains("sweeping") && !text.contains("reading"),
            "about panel must not claim another animation set"
        );
    }

    #[test]
    fn missing_about_frames_show_diagnostic_not_sweep_frames() {
        use crate::akari::animation::AnimationSet;

        let mut app = crate::app::App::new(AppCommand::About);
        let assets = app.sprites.get_mut(AnimationSet::About);
        assets.frames.clear();
        assets.error = Some("missing sprite frame 1 for about set".to_string());
        let text = buffer_text(120, 30, &app);
        assert!(text.contains("investigating"));
        assert!(text.contains("missing sprite frame 1 for about set"));
        assert!(!text.contains("sweeping"));
    }

    #[test]
    fn sweep_panel_still_reports_maintenance_caption() {
        let app = crate::app::App::new(AppCommand::Clean);
        let text = buffer_text(100, 28, &app);
        assert!(text.contains("sweeping"), "running sweep caption preserved");
    }

    #[test]
    fn all_phase_transition_replaces_sweep_frame_with_health_frame() {
        use crate::akari::animation::AnimationSet;
        use crate::akari::sprites::SpriteFrame;

        let mut app = crate::app::App::new(AppCommand::All);
        app.sprites.get_mut(AnimationSet::Sweep).error = None;
        app.sprites.get_mut(AnimationSet::Sweep).frames = vec![SpriteFrame {
            width: 6,
            height: 8,
            rgba: vec![255, 0, 0, 255].repeat(48),
        }];
        app.sprites.get_mut(AnimationSet::Health).error = None;
        app.sprites.get_mut(AnimationSet::Health).frames = vec![SpriteFrame {
            width: 2,
            height: 4,
            rgba: vec![0, 255, 0, 255].repeat(8),
        }];

        let cleanup = buffer_text(160, 40, &app);
        assert!(cleanup.contains("6×8 source"), "cleanup frame must show");
        assert!(cleanup.contains("sweeping"));

        app.on_event(crate::system::ExecEvent::PhaseStarted {
            name: "cleanup".into(),
        });
        let cleanup_phase = buffer_text(160, 40, &app);
        assert!(cleanup_phase.contains("6×8 source"));

        app.on_event(crate::system::ExecEvent::PhaseStarted {
            name: "repair".into(),
        });
        let repair = buffer_text(160, 40, &app);
        assert!(repair.contains("2×4 source"), "health frame must show");
        assert!(repair.contains("repairing"));
        assert!(
            !repair.contains("6×8 source") && !repair.contains("sweeping"),
            "stale sweep frame/caption must not survive the phase switch"
        );
    }

    #[test]
    fn health_panel_reports_health_caption_and_source_dimensions() {
        use crate::akari::sprites::SpriteFrame;

        let mut app = crate::app::App::new(AppCommand::Health);
        assert_eq!(
            app.animator.set(),
            crate::akari::animation::AnimationSet::Health
        );
        let assets = app
            .sprites
            .get_mut(crate::akari::animation::AnimationSet::Health);
        assets.error = None;
        assets.frames = vec![SpriteFrame {
            width: 2,
            height: 4,
            rgba: vec![255, 0, 0, 255].repeat(8),
        }];
        let text = buffer_text(160, 40, &app);
        assert!(
            text.contains("repairing"),
            "health caption must report health work"
        );
        assert!(text.contains("2×4 source"), "caption must use frame size");
        assert!(
            !text.contains("sweeping") && !text.contains("256×320"),
            "health caption must not claim sweep activity or dimensions"
        );
    }

    #[test]
    fn missing_health_frames_show_diagnostic_not_sweep_frames() {
        let mut app = crate::app::App::new(AppCommand::Health);
        let assets = app
            .sprites
            .get_mut(crate::akari::animation::AnimationSet::Health);
        assets.frames.clear();
        assets.error = Some("missing sprite frame 1 for health set".to_string());
        let text = buffer_text(120, 30, &app);
        assert!(text.contains("repairing"));
        assert!(text.contains("missing sprite frame 1 for health set"));
        assert!(!text.contains("sweeping"));
    }

    #[test]
    fn tiny_terminal_degrades_gracefully() {
        let app = crate::app::App::new(AppCommand::Home);
        let text = buffer_text(40, 10, &app);
        assert!(text.contains(PRODUCT_LABEL));
    }

    #[test]
    fn placeholder_visible_only_when_input_empty() {
        assert!(placeholder_visible(""));
        assert!(!placeholder_visible("a"));
        assert!(!placeholder_visible("back"));
        assert!(!placeholder_visible("-"));
        assert!(!placeholder_visible("akariclean"));
    }

    #[test]
    fn empty_entry_shows_hint_typing_hides_it() {
        // Empty: instructional hint visible.
        let app = crate::app::App::new(AppCommand::Home);
        let text = buffer_text(100, 28, &app);
        assert!(
            text.contains("type 1-5 or press /"),
            "empty entry must show placeholder hint"
        );
        // Typing the first character hides the placeholder immediately:
        // input text present, hint absent, no pushing/overlap.
        let mut typing = crate::app::App::new(AppCommand::Home);
        typing.set_input_focus(true);
        typing.push_input("a");
        let text2 = buffer_text(100, 28, &typing);
        assert!(text2.contains("> a"), "typed text must appear normally");
        assert!(
            !text2.contains("type 1-5 or press /"),
            "placeholder must hide once typing begins"
        );
        assert!(
            !text2.contains("[Enter] run"),
            "focused hint must also hide while typing"
        );
    }

    #[test]
    fn clearing_input_restores_placeholder() {
        let mut app = crate::app::App::new(AppCommand::Home);
        app.set_input_focus(true);
        app.push_input("akariclean");
        assert!(!placeholder_visible(&app.input));
        let text_full = buffer_text(100, 28, &app);
        assert!(!text_full.contains("type 1-5 or press /"));
        app.clear_input();
        assert!(placeholder_visible(&app.input));
        let text_cleared = buffer_text(100, 28, &app);
        assert!(
            text_cleared.contains("[Enter] run"),
            "clearing must restore the focused placeholder"
        );
    }

    #[test]
    fn deletion_and_cursor_do_not_break_entry_rendering() {
        let mut app = crate::app::App::new(AppCommand::Home);
        app.set_input_focus(true);
        app.push_input("akari");
        app.backspace_input();
        assert_eq!(app.input, "akar");
        let text = buffer_text(100, 28, &app);
        assert!(text.contains("> akar"), "edited text must render");
        // Placeholder still hidden while non-empty.
        assert!(!text.contains("type 1-5 or press /"));
        app.clear_input();
        let text2 = buffer_text(100, 28, &app);
        assert!(text2.contains("[Enter] run"));
    }

    #[test]
    fn no_press_1_to_exit_and_back_hint_present() {
        for cmd in [
            AppCommand::Home,
            AppCommand::Clean,
            AppCommand::Health,
            AppCommand::All,
        ] {
            let app = crate::app::App::new(cmd);
            let text = buffer_text(100, 28, &app);
            assert!(
                !text.contains("Press 1 to exit") && !text.contains("press 1 to exit"),
                "old ambiguous exit instruction must be gone"
            );
        }
        // Maintenance Done shows the new unambiguous navigation.
        let mut done = crate::app::App::new(AppCommand::Clean);
        done.execution = crate::app::ExecutionState::Done;
        done.summary = Some(crate::system::AppSummary::compute(
            "completed".into(),
            "not run - cleanup only".into(),
            false,
            crate::system::RunTotals::default(),
            vec![],
            vec![],
            None,
            None,
        ));
        let text = buffer_text(120, 30, &done);
        assert!(
            text.contains("back") && text.contains("akariback") || text.contains("-"),
            "done screen must advertise back/akariback/- navigation"
        );
        // Status bar advertises -/back without claiming 1 exits.
        assert!(status_text(&done).contains("-/back"));
    }

    #[test]
    fn running_status_hides_idle_hints_but_keeps_progress() {
        use crate::system::ProgressState;

        // Idle: the normal status text with keyboard hints stays visible.
        let idle = crate::app::App::new(AppCommand::Home);
        let idle_text = status_text(&idle);
        assert!(idle_text.contains("1-5 select"), "idle keeps hints");
        assert!(idle_text.contains("/ input"));
        assert!(idle_text.contains("-/back home"));
        assert!(idle_text.contains("q/Esc quit"));

        // Running (cleanup, repair, combined): progress display only —
        // no idle keyboard hints and no idle badge, while progress
        // information and the running indicator stay visible.
        for cmd in [AppCommand::Clean, AppCommand::Health, AppCommand::All] {
            let mut running = crate::app::App::new(cmd);
            running.screen = crate::app::Screen::Maintenance;
            running.execution = crate::app::ExecutionState::Running;
            running.progress = ProgressState::new(2, 6, "working…".to_string());
            let s = status_text(&running);
            assert!(s.contains("Status:"), "progress visible ({cmd:?}): {s}");
            assert!(s.contains('%'), "percentage visible ({cmd:?}): {s}");
            assert!(s.contains("[running|"), "running indicator kept: {s}");
            for hint in [
                "1-5 select",
                "/ input",
                "-/back home",
                "q/Esc quit",
                "q quit",
                "[idle|",
            ] {
                assert!(!s.contains(hint), "running hides {hint:?} ({cmd:?}): {s}");
            }
        }

        // Cancelled progress while still running: same progress-only bar.
        let mut cancelled = crate::app::App::new(AppCommand::Clean);
        cancelled.screen = crate::app::Screen::Maintenance;
        cancelled.execution = crate::app::ExecutionState::Running;
        let mut p = ProgressState::new(2, 6, "DISM ScanHealth".to_string());
        p.cancelled = true;
        cancelled.progress = p;
        let s = status_text(&cancelled);
        assert!(s.contains("cancelled"), "cancellation shown: {s}");
        assert!(!s.contains("1-5 select") && !s.contains("-/back home"));

        // Settled states restore the navigation hints.
        for exec in [
            crate::app::ExecutionState::Done,
            crate::app::ExecutionState::Error,
        ] {
            let mut settled = crate::app::App::new(AppCommand::Clean);
            settled.screen = crate::app::Screen::Maintenance;
            settled.execution = exec;
            settled.progress = ProgressState::new(6, 6, "done".to_string());
            let s = status_text(&settled);
            assert!(s.contains("Status:"), "progress kept ({exec:?}): {s}");
            assert!(
                s.contains("-/back home") && s.contains("q quit"),
                "settled restores hints ({exec:?}): {s}"
            );
        }
    }

    #[test]
    fn removed_shortcut_hints_no_longer_appear() {
        // Removed single-key hints (`c`, `r`, `?`, `a`), the `exit=`
        // status display, and the retired one-means-exit notice must be
        // gone from every rendered screen; retained navigation
        // (1-5, /, -/back, q/Esc) stays.
        for cmd in [
            AppCommand::Home,
            AppCommand::Clean,
            AppCommand::Health,
            AppCommand::All,
            AppCommand::About,
            AppCommand::Help,
        ] {
            let app = crate::app::App::new(cmd);
            let text = buffer_text(120, 30, &app);
            assert!(
                !text.contains("c clean"),
                "c clean hint must be gone ({cmd:?}): {text}"
            );
            assert!(
                !text.contains("r repair"),
                "r repair hint must be gone ({cmd:?})"
            );
            assert!(
                !text.contains("? help"),
                "? help hint must be gone ({cmd:?})"
            );
            assert!(
                !text.contains("press ?"),
                "press ? hint must be gone ({cmd:?})"
            );
            assert!(
                !text.contains("c re-runs"),
                "c re-runs hint must be gone ({cmd:?})"
            );
            assert!(
                !text.contains("a about"),
                "a about hint must be gone ({cmd:?}): {text}"
            );
            assert!(
                !text.contains("exit="),
                "exit= display must be gone ({cmd:?}): {text}"
            );
            assert!(
                !text.contains("never means exit"),
                "never-means-exit message must be gone ({cmd:?}): {text}"
            );
            // Help screen must not advertise removed single-key reruns.
            if cmd == AppCommand::Help {
                assert!(
                    !text.contains("r repair"),
                    "help must not advertise r repair"
                );
            }
        }
        // Status bars never advertise removed shortcuts and never show
        // the exit-code display.
        for cmd in [
            AppCommand::Home,
            AppCommand::About,
            AppCommand::Help,
            AppCommand::Clean,
            AppCommand::Health,
            AppCommand::All,
        ] {
            let app = crate::app::App::new(cmd);
            let s = status_text(&app);
            assert!(!s.contains("c clean"), "status must not show c clean");
            assert!(!s.contains("r repair"), "status must not show r repair");
            assert!(!s.contains("? help"), "status must not show ? help");
            assert!(!s.contains("a about"), "status must not show a about");
            assert!(!s.contains("exit="), "status must not show exit=");
        }
        // Startup log and home/help content carry no removed hints.
        let home = crate::app::App::new(AppCommand::Home);
        for line in &home.log {
            assert!(
                !line.contains("press ?"),
                "startup log must not say press ?"
            );
        }
        for line in crate::ui::home::lines() {
            let s = format!("{line:?}");
            assert!(!s.contains("? help"), "home must not show ? help: {s}");
        }
        for line in crate::ui::help::lines() {
            let s = format!("{line:?}");
            assert!(
                !s.contains("c re-runs") && !s.contains("r repair"),
                "help must not advertise c/r reruns: {s}"
            );
        }
    }
}
