//! Maintenance screen helpers: progress + summary text.
use crate::app::App;
use crate::system::OpStatus;
use ratatui::text::Line;

/// Fixed bar width matching the spec example
/// (`[====================------]` = 26 cells).
pub const PROGRESS_BAR_WIDTH: usize = 26;

/// Visual progress bar + numerical percentage (one decimal place) for the
/// Status panel. Task-count approximation: `≈` marks estimated
/// task-based progress (never exact time/bytes); indeterminate work shows
/// an explicit working marker instead of a fabricated percentage.
/// Failures render as `FAILED`, never as plain success; cancellation
/// renders as `cancelled` below 100%.
pub fn progress_bar_line(app: &App) -> String {
    if app.progress.total == 0 {
        return match app.execution {
            crate::app::ExecutionState::Running => {
                if app.progress.current.is_empty() {
                    "Status: [··· working ···] starting…".to_string()
                } else {
                    format!("Status: [··· working ···] {}", app.progress.current)
                }
            }
            _ => "Status: no operations yet".to_string(),
        };
    }
    // Prefix task-based percentages with `≈` to distinguish estimates
    // from exact progress (no task here exposes exact intermediate
    // progress; all advances are completed-unit counts).
    let base = app.progress.status_line(PROGRESS_BAR_WIDTH);
    // `status_line` yields `Status: [bar] 76.4% …`; insert the estimate
    // marker before the numeric percentage for honesty.
    mark_estimate(base)
}

fn mark_estimate(line: String) -> String {
    // Turn `] 76.4%` into `] ≈76.4% (tasks)` unless the line is already
    // indeterminate/terminally labelled. Keeps the spec example shape
    // while labelling the approximation.
    if line.contains("indeterminate") {
        return line;
    }
    if let Some(pos) = line.find("] ") {
        let (head, tail) = line.split_at(pos + 2);
        // tail starts with e.g. `76.4% …`
        if let Some(pct_end) = tail.find('%') {
            let (num, rest) = tail.split_at(pct_end + 1);
            // Terminal states already carry done/FAILED/cancelled labels;
            // still mark the number as approximate.
            return format!("{head}≈{num} (tasks){rest}");
        }
    }
    line
}

pub fn progress_line(app: &App) -> String {
    let total = app.operations.len();
    if total == 0 {
        return match app.execution {
            crate::app::ExecutionState::Running => "starting…".to_string(),
            _ => "no operations yet".to_string(),
        };
    }
    let done = app
        .operations
        .iter()
        .filter(|o| {
            matches!(
                o.status,
                OpStatus::Success | OpStatus::Warning | OpStatus::Failed | OpStatus::Skipped
            )
        })
        .count();
    let failed = app
        .operations
        .iter()
        .filter(|o| o.status == OpStatus::Failed)
        .count();
    let warned = app
        .operations
        .iter()
        .filter(|o| o.status == OpStatus::Warning)
        .count();
    format!("{done}/{total} settled · {failed} failed · {warned} warnings")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::AppCommand;
    use crate::system::ProgressState;

    fn bar_app(
        completed: u64,
        total: u64,
        failed: bool,
        indeterminate: bool,
        current: &str,
    ) -> App {
        let mut app = App::new(AppCommand::Clean);
        app.screen = crate::app::Screen::Maintenance;
        app.execution = crate::app::ExecutionState::Running;
        let mut p = ProgressState::new(completed, total, current.to_string());
        p.failed = failed;
        p.indeterminate = indeterminate;
        app.progress = p;
        app
    }

    #[test]
    fn bar_matches_spec_shape_with_one_decimal() {
        let app = bar_app(764, 1000, false, false, "cleaning");
        let line = progress_bar_line(&app);
        assert!(line.starts_with("Status: ["), "{line}");
        assert!(line.contains("76.4%"), "{line}");
        // 26-cell bar with =/- halves.
        let start = line.find('[').unwrap() + 1;
        let end = line.find(']').unwrap();
        assert_eq!(end - start, PROGRESS_BAR_WIDTH);
        let bar = &line[start..end];
        assert!(bar.contains('=') && bar.contains('-'));
    }

    #[test]
    fn bar_holds_working_without_advancing() {
        let app = bar_app(2, 6, false, true, "DISM RestoreHealth");
        let line = progress_bar_line(&app);
        assert!(line.contains("33.3%"), "{line}");
        assert!(line.contains("working:"), "{line}");
    }

    #[test]
    fn bar_failure_is_distinguishable() {
        let app = bar_app(6, 6, true, false, "done");
        let line = progress_bar_line(&app);
        assert!(line.contains("100.0%"), "{line}");
        assert!(line.contains("FAILED"), "{line}");
        let ok = bar_app(6, 6, false, false, "");
        assert!(!progress_bar_line(&ok).contains("FAILED"));
    }

    #[test]
    fn bar_unknown_total_is_indeterminate() {
        let mut app = App::new(AppCommand::Clean);
        app.screen = crate::app::Screen::Maintenance;
        app.execution = crate::app::ExecutionState::Running;
        let line = progress_bar_line(&app);
        assert!(
            line.contains("indeterminate") || line.contains("starting"),
            "{line}"
        );
        assert!(!line.contains("100.0%"));
    }
}

pub fn summary_lines(app: &App) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    if let Some(s) = &app.summary {
        out.push(Line::from(format!("Overall : {}", s.overall)));
        out.push(Line::from(format!("Cleanup : {}", s.cleanup_status)));
        out.push(Line::from(format!("Repair  : {}", s.repair_status)));
        out.push(Line::from(format!(
            "Dirs ok/missing: {}/{} · files removed: {} ({} KB) · left: {}",
            s.totals.dirs_done,
            s.totals.dirs_missing,
            s.totals.files_removed,
            s.totals.kb_removed,
            s.totals.files_left
        )));
        out.push(Line::from(format!(
            "Free: {} → {} (net {})",
            crate::system::format_kb(s.free_before_kb),
            crate::system::format_kb(s.free_after_kb),
            match (s.free_before_kb, s.free_after_kb) {
                (Some(a), Some(b)) => crate::system::format_kb(Some(b - a)),
                _ => "unavailable".to_string(),
            }
        )));
        // Unambiguous return navigation once the run settled.
        if matches!(
            app.execution,
            crate::app::ExecutionState::Done | crate::app::ExecutionState::Error
        ) {
            out.push(Line::from(
                "Type back, akariback, or press - to return to the main menu.",
            ));
        }
    } else if matches!(
        app.execution,
        crate::app::ExecutionState::Done | crate::app::ExecutionState::Error
    ) {
        out.push(Line::from(
            "Type back, akariback, or press - to return to the main menu.",
        ));
    }
    out
}
