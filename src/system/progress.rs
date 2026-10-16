//! Weighted, evidence-based progress model for maintenance operations.
//!
//! The model is task-count based (documented approximation, not exact time
//! or byte progress) because opaque Windows utilities expose no trustworthy
//! intermediate progress and filesystem totals are unknown without an
//! expensive extra traversal (deliberately avoided).
//!
//! - Cleaning: one work unit per directory target / program step that
//!   produces an `OpResult`. Dynamic Chromium/Firefox profile subdirs are
//!   counted via a cheap shallow scan (existence + one `read_dir` per
//!   root), never a full recursive walk.
//! - Health: fixed 6 steps (`repair_plan` order). Completed steps advance;
//!   the running DISM/SFC/CHKDSK holds at the last defensible percentage
//!   with a `working…` marker — no fabricated intra-command percentages,
//!   no log-line parsing as progress.
//! - Combined (`akariall`): shared total `cleanup_total + 6`, completed
//!   accumulates across phases, never resets, never moves backward.
//! - Failures: reaching `completed == total` with any failure renders as
//!   `FAILED`, never as unconditional success. Cancellation leaves progress
//!   below 100% with an explicit cancelled marker.

/// One snapshot of operation-level progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressState {
    /// Work units finished (evidence-backed completions only).
    pub completed: u64,
    /// Total work units for this operation (stable per run).
    pub total: u64,
    /// Human-readable current task (e.g. banner or program name).
    pub current: String,
    /// True once any required task has failed (sticky, never clears
    /// back to success within the same run).
    pub failed: bool,
    /// True when the run was cancelled before reaching its terminal state.
    pub cancelled: bool,
    /// True while the current task is opaque and its remaining work cannot
    /// be estimated — the bar holds at the last defensible percentage and
    /// the UI shows a working indicator instead of advancing.
    pub indeterminate: bool,
}

impl ProgressState {
    pub fn new(completed: u64, total: u64, current: String) -> Self {
        Self {
            completed,
            total,
            current,
            failed: false,
            cancelled: false,
            indeterminate: false,
        }
    }

    /// Fraction 0.0..=1.0. `total == 0` yields 0.0 (explicitly unknown,
    /// never fabricated as 100%).
    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        let c = self.completed.min(self.total) as f64;
        (c / self.total as f64).clamp(0.0, 1.0)
    }

    /// Percentage 0.0..=100.0.
    pub fn percent(&self) -> f64 {
        self.fraction() * 100.0
    }

    /// Percentage with exactly one decimal place, e.g. `76.4%`.
    pub fn percent_text(&self) -> String {
        format!("{:.1}%", self.percent())
    }

    /// Render `[====----]` bar of `width` cells. Uses `=` for finished
    /// work and `-` for remaining, matching the spec example
    /// `Status: [====================------] 76.4%`.
    pub fn bar(&self, width: usize) -> String {
        render_bar(self.completed, self.total, width)
    }

    /// Full status line for the TUI Status panel, e.g.
    /// `Status: [============--------] 62.5% (working: DISM RestoreHealth…)`.
    ///
    /// - Success terminal: `… 100.0% done`.
    /// - Failure terminal: `… 100.0% FAILED` (visually distinct, never
    ///   shown as plain success).
    /// - Cancelled: `… 42.1% cancelled`.
    /// - Indeterminate working: appends `(working: <current>)`.
    /// - Unknown total (`total == 0`): explicit indeterminate marker, no
    ///   fabricated percentage.
    pub fn status_line(&self, width: usize) -> String {
        if self.total == 0 {
            let cur = if self.current.is_empty() {
                "working…".to_string()
            } else {
                format!("working: {}", self.current)
            };
            return format!("Status: [··· working ···] indeterminate — {cur}");
        }
        let bar = self.bar(width);
        let pct = self.percent_text();
        if self.cancelled {
            return format!("Status: [{bar}] {pct} cancelled — {}", self.current);
        }
        if self.failed && self.completed >= self.total {
            return format!("Status: [{bar}] {pct} FAILED — {}", self.current);
        }
        if self.failed {
            if self.indeterminate && !self.current.is_empty() {
                return format!("Status: [{bar}] {pct} FAILED (working: {})", self.current);
            }
            if self.current.is_empty() {
                return format!("Status: [{bar}] {pct} FAILED");
            }
            return format!("Status: [{bar}] {pct} FAILED — {}", self.current);
        }
        if self.indeterminate && !self.current.is_empty() {
            return format!("Status: [{bar}] {pct} (working: {})", self.current);
        }
        if self.completed >= self.total {
            return format!("Status: [{bar}] {pct} done");
        }
        if self.current.is_empty() {
            return format!("Status: [{bar}] {pct}");
        }
        format!("Status: [{bar}] {pct} — {}", self.current)
    }
}

impl Default for ProgressState {
    fn default() -> Self {
        Self {
            completed: 0,
            total: 0,
            current: String::new(),
            failed: false,
            cancelled: false,
            indeterminate: false,
        }
    }
}

/// Pure bar renderer: `completed/total` mapped onto `width` cells.
/// Clamps over-completion, handles `total == 0` as fully empty (callers
/// show the indeterminate marker instead of a percentage).
pub fn render_bar(completed: u64, total: u64, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if total == 0 {
        return "-".repeat(width);
    }
    let frac = (completed.min(total) as f64 / total as f64).clamp(0.0, 1.0);
    let filled = (frac * width as f64).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "=".repeat(filled), "-".repeat(empty))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_matches_spec_example_shape() {
        // 76.4% over 26 cells ≈ 20 filled, 6 empty.
        let bar = render_bar(764, 1000, 26);
        assert_eq!(bar.len(), 26);
        assert!(bar.starts_with("===================="));
        assert!(bar.ends_with("------"));
        assert!(bar.contains('=') && bar.contains('-'));
    }

    #[test]
    fn percent_has_one_decimal_place() {
        let s = ProgressState::new(764, 1000, String::new());
        assert_eq!(s.percent_text(), "76.4%");
        let zero = ProgressState::new(0, 10, String::new());
        assert_eq!(zero.percent_text(), "0.0%");
        let full = ProgressState::new(10, 10, String::new());
        assert_eq!(full.percent_text(), "100.0%");
    }

    #[test]
    fn zero_total_is_indeterminate_not_fabricated() {
        let s = ProgressState::new(0, 0, "DISM RestoreHealth".to_string());
        assert_eq!(s.fraction(), 0.0);
        let line = s.status_line(20);
        assert!(line.contains("indeterminate"));
        assert!(!line.contains("100.0%"));
        assert!(line.contains("DISM RestoreHealth"));
    }

    #[test]
    fn failure_terminal_is_distinguishable_from_success() {
        let mut failed = ProgressState::new(6, 6, "done".to_string());
        failed.failed = true;
        let line = failed.status_line(10);
        assert!(line.contains("100.0%"));
        assert!(line.contains("FAILED"));
        let ok = ProgressState::new(6, 6, String::new());
        let ok_line = ok.status_line(10);
        assert!(ok_line.contains("done"));
        assert!(!ok_line.contains("FAILED"));
        assert_ne!(line, ok_line);
    }

    #[test]
    fn failure_below_total_does_not_claim_completion() {
        let mut s = ProgressState::new(3, 6, "SFC".to_string());
        s.failed = true;
        let line = s.status_line(10);
        assert!(line.contains("50.0%"));
        assert!(line.contains("FAILED"));
        assert!(!line.contains("done"));
    }

    #[test]
    fn cancelled_is_explicit_and_below_full() {
        let mut s = ProgressState::new(2, 6, "DISM ScanHealth".to_string());
        s.cancelled = true;
        let line = s.status_line(10);
        assert!(line.contains("cancelled"));
        assert!(!line.contains("100.0%"));
    }

    #[test]
    fn indeterminate_holds_percentage_and_names_task() {
        let mut s = ProgressState::new(2, 6, "DISM RestoreHealth".to_string());
        s.indeterminate = true;
        let line = s.status_line(12);
        // Percentage reflects completed work only (2/6 = 33.3%).
        assert!(line.contains("33.3%"));
        assert!(line.contains("working: DISM RestoreHealth"));
    }

    #[test]
    fn progress_never_exceeds_total() {
        let s = ProgressState::new(99, 6, String::new());
        assert_eq!(s.fraction(), 1.0);
        assert_eq!(s.bar(10), "=".repeat(10));
    }

    #[test]
    fn empty_width_bar_is_empty() {
        assert_eq!(render_bar(5, 10, 0), "");
    }
}
