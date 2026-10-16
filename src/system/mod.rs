//! Shared maintenance types, runner abstraction, and summary logic.
//!
//! Mirrors the bookkeeping in `Akari-Sweeps.bat` (`OK_COUNT`, `ERR_COUNT`,
//! `WARN_COUNT`, `OVERALL_RC`) so tests can verify orchestration without
//! touching the real filesystem or Windows servicing stack.

pub mod elevation;
pub mod process;
pub mod windows;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

/// Lifecycle of a single maintenance operation as shown in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum OpStatus {
    Pending,
    Running,
    Success,
    /// Ran, but the result is unknown or needs attention (e.g. an
    /// exit code the OS does not document). Counts as a warning, never
    /// as an error — exit codes only reflect `Failed`.
    Warning,
    Failed,
    Skipped,
}

impl fmt::Display for OpStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            OpStatus::Pending => "pending",
            OpStatus::Running => "running",
            OpStatus::Success => "ok",
            OpStatus::Warning => "warning",
            OpStatus::Failed => "FAILED",
            OpStatus::Skipped => "skipped",
        };
        write!(f, "{s}")
    }
}

/// Structured result for one operation. Never infer success from a log line;
/// the exit code / filesystem measurement decides.
#[derive(Debug, Clone)]
pub struct OpResult {
    pub id: String,
    pub label: String,
    pub target: String,
    pub status: OpStatus,
    pub exit_code: Option<i32>,
    pub detail: String,
    pub files_removed: u64,
    pub kb_removed: u64,
    pub files_left: u64,
}

impl OpResult {
    pub fn ok(id: &str, label: &str, target: &str, detail: String) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            target: target.to_string(),
            status: OpStatus::Success,
            exit_code: None,
            detail,
            files_removed: 0,
            kb_removed: 0,
            files_left: 0,
        }
    }

    pub fn failed(id: &str, label: &str, target: &str, detail: String) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            target: target.to_string(),
            status: OpStatus::Failed,
            exit_code: None,
            detail,
            files_removed: 0,
            kb_removed: 0,
            files_left: 0,
        }
    }

    pub fn skipped(id: &str, label: &str, target: &str, detail: String) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            target: target.to_string(),
            status: OpStatus::Skipped,
            exit_code: None,
            detail,
            files_removed: 0,
            kb_removed: 0,
            files_left: 0,
        }
    }

    pub fn warning(id: &str, label: &str, target: &str, detail: String) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            target: target.to_string(),
            status: OpStatus::Warning,
            exit_code: None,
            detail,
            files_removed: 0,
            kb_removed: 0,
            files_left: 0,
        }
    }
}

/// Outcome of a `CleanDir` call, mirroring `:CleanDir` / `:TallyDir`.
#[derive(Debug, Clone)]
pub struct DirOutcome {
    /// False when the directory did not exist (BAT counts `DIRS_MISSING`).
    pub existed: bool,
    pub files_removed: u64,
    pub kb_removed: u64,
    pub files_left: u64,
    pub measure_failed: bool,
    /// Breakdown of why `files_left` remain, where the runner could
    /// determine it. `locked` = sharing/lock violation (in use);
    /// `permission_denied` = access denied; `other_unremovable` = any
    /// other I/O failure. Best-effort: sums to ≤ `files_left`.
    pub locked_skipped: u64,
    pub permission_denied: u64,
    pub other_unremovable: u64,
}

/// Outcome of an external program run.
#[derive(Debug, Clone)]
pub struct ProgramOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Events streamed from the background worker to the UI thread.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ExecEvent {
    Log(String),
    OpStarted {
        id: String,
        label: String,
        target: String,
    },
    OpProgress {
        id: String,
        line: String,
    },
    OpFinished(OpResult),
    PhaseStarted {
        name: String,
    },
    PhaseFinished {
        name: String,
    },
    Finished(AppSummary),
    Error(String),
}

/// Abstraction over side effects so orchestration is unit-testable.
/// The real implementation lives in `commands::*` (`RealRunner`);
/// tests substitute a mock.
pub trait MaintenanceRunner {
    fn clean_dir(&mut self, path: &Path) -> DirOutcome;
    fn run_program(&mut self, program: &str, args: &[&str]) -> ProgramOutcome;
    fn program_exists(&mut self, program: &str) -> bool;
    fn emit(&mut self, event: ExecEvent);
}

pub type EventTx = Sender<ExecEvent>;

/// Aggregate counts mirroring the BAT summary variables.
#[derive(Debug, Clone, Default)]
pub struct RunTotals {
    pub ok_count: u64,
    pub err_count: u64,
    pub warn_count: u64,
    pub dirs_done: u64,
    pub dirs_missing: u64,
    pub files_removed: u64,
    pub kb_removed: u64,
    pub files_left: u64,
    pub measure_fail: u64,
    /// Best-effort breakdown of `files_left` by cause. Populated from
    /// per-file delete-error classification; never force-deletes.
    pub files_locked: u64,
    pub files_permission: u64,
    pub files_other_left: u64,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

impl RunTotals {
    pub fn add_warning(&mut self, msg: String) {
        self.warn_count += 1;
        self.warnings.push(msg);
    }

    pub fn add_error(&mut self, msg: String) {
        self.err_count += 1;
        self.errors.push(msg);
    }

    pub fn add_ok(&mut self) {
        self.ok_count += 1;
    }
}

/// Final summary shown in the TUI and used for the process exit code.
/// Exit-code rule mirrors `:ShowSummary`:
/// errors + some success => 1, errors + no success => 2, else 0.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AppSummary {
    pub cleanup_status: String,
    pub repair_status: String,
    pub repair_entered: bool,
    pub totals: RunTotals,
    pub cleanup_results: Vec<OpResult>,
    pub repair_results: Vec<OpResult>,
    pub free_before_kb: Option<i64>,
    pub free_after_kb: Option<i64>,
    pub overall: String,
    pub exit_code: i32,
}

impl AppSummary {
    pub fn compute(
        cleanup_status: String,
        repair_status: String,
        repair_entered: bool,
        totals: RunTotals,
        cleanup_results: Vec<OpResult>,
        repair_results: Vec<OpResult>,
        free_before_kb: Option<i64>,
        free_after_kb: Option<i64>,
    ) -> Self {
        let mut overall;
        let mut exit_code = 0;
        if cleanup_status == "completed" {
            overall = "COMPLETED SUCCESSFULLY".to_string();
            if totals.warn_count > 0 {
                overall = "COMPLETED WITH WARNINGS".to_string();
            }
        } else {
            overall = "NOT RUN - no changes were made".to_string();
        }
        if totals.err_count > 0 {
            if totals.ok_count > 0 {
                overall = format!(
                    "COMPLETED PARTIALLY - failed operations: {}",
                    totals.err_count
                );
                exit_code = 1;
            } else {
                overall = "ENCOUNTERED AN ERROR - no operation succeeded".to_string();
                exit_code = 2;
            }
        }
        Self {
            cleanup_status,
            repair_status,
            repair_entered,
            totals,
            cleanup_results,
            repair_results,
            free_before_kb,
            free_after_kb,
            overall,
            exit_code,
        }
    }
}

/// Format a KB value like the BAT `:FormatKB` helper.
pub fn format_kb(kb: Option<i64>) -> String {
    match kb {
        None => "unavailable".to_string(),
        Some(k) => {
            let v = k as f64;
            if v.abs() >= 1_048_576.0 {
                format!("{:.2} GB", v / 1_048_576.0)
            } else if v.abs() >= 1024.0 {
                format!("{:.2} MB", v / 1024.0)
            } else {
                format!("{k} KB")
            }
        }
    }
}

/// Expand a `%VAR%\suffix` Windows template using process env vars.
/// Returns `None` when a referenced variable is missing.
pub fn expand_windows_template(template: &str) -> Option<PathBuf> {
    let mut out = String::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let Some(end) = template[i + 1..].find('%') {
                let var = &template[i + 1..i + 1 + end];
                match std::env::var(var) {
                    Ok(val) => out.push_str(&val),
                    Err(_) => return None,
                }
                i += end + 2;
                continue;
            }
            return None;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Some(PathBuf::from(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_partial_when_some_ok_and_errors() {
        let mut t = RunTotals::default();
        t.add_ok();
        t.add_error("boom".into());
        let s = AppSummary::compute(
            "completed".into(),
            "skipped by user".into(),
            false,
            t,
            vec![],
            vec![],
            None,
            None,
        );
        assert_eq!(s.exit_code, 1);
        assert!(s.overall.contains("PARTIALLY"));
    }

    #[test]
    fn exit_code_two_when_no_success() {
        let mut t = RunTotals::default();
        t.add_error("boom".into());
        let s = AppSummary::compute(
            "not run".into(),
            "not offered".into(),
            false,
            t,
            vec![],
            vec![],
            None,
            None,
        );
        assert_eq!(s.exit_code, 2);
    }

    #[test]
    fn exit_code_zero_on_clean_success() {
        let mut t = RunTotals::default();
        t.add_ok();
        let s = AppSummary::compute(
            "completed".into(),
            "skipped by user".into(),
            false,
            t,
            vec![],
            vec![],
            None,
            None,
        );
        assert_eq!(s.exit_code, 0);
        assert_eq!(s.overall, "COMPLETED SUCCESSFULLY");
    }

    #[test]
    fn warnings_change_overall_but_not_exit_code() {
        let mut t = RunTotals::default();
        t.add_ok();
        t.add_warning("locked file".into());
        let s = AppSummary::compute(
            "completed".into(),
            "not offered".into(),
            false,
            t,
            vec![],
            vec![],
            None,
            None,
        );
        assert_eq!(s.exit_code, 0);
        assert_eq!(s.overall, "COMPLETED WITH WARNINGS");
    }

    #[test]
    fn format_kb_units() {
        assert_eq!(format_kb(None), "unavailable");
        assert_eq!(format_kb(Some(512)), "512 KB");
        assert!(format_kb(Some(2048)).contains("MB"));
        assert!(format_kb(Some(2_097_152)).contains("GB"));
    }
}
