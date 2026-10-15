//! Maintenance screen helpers: progress + summary text.
use crate::app::App;
use crate::system::OpStatus;
use ratatui::text::Line;

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
    }
    out
}
