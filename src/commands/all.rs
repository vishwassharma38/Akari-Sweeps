//! Combined workflow — `akariall`: cleanup followed by repair.
//!
//! The BAT gates repair behind an interactive Y/N prompt. An explicit
//! `akariall` invocation *is* that consent, so both phases run in order
//! without a second prompt; the final summary still reports each phase
//! honestly (a failed cleanup does not silently cancel repair, and the
//! overall status is never "success" when a required phase failed).

use crate::commands::clean::run_cleanup_phase;
use crate::commands::health::run_repair_phase;
use crate::system::windows::free_space_kb;
use crate::system::{AppSummary, ExecEvent, MaintenanceRunner, RunTotals};

/// Run cleanup, then repair, merging totals like the BAT summary.
/// `repair_already_entered` is the `REPAIR_ENTERED` guard (always false on
/// a fresh `akariall` run; exposed for tests).
pub fn run_all_phases(
    runner: &mut dyn MaintenanceRunner,
    repair_already_entered: bool,
) -> AppSummary {
    let free_before = free_space_kb();

    runner.emit(ExecEvent::Log(
        "akariall: running cleanup followed by system-image repair.".to_string(),
    ));

    // Phase 1: cleanup (always runs).
    let (cleanup_results, mut totals) = run_cleanup_phase(runner);
    let free_after = free_space_kb();

    // Phase 2: repair (runs after cleanup; never concurrent).
    let err_at_repair_start = totals.err_count;
    let (repair_results, repair_totals, entered) = run_repair_phase(runner, repair_already_entered);

    // Merge repair bookkeeping into the shared totals.
    let merged = RunTotals {
        ok_count: totals.ok_count + repair_totals.ok_count,
        err_count: totals.err_count + repair_totals.err_count,
        warn_count: totals.warn_count + repair_totals.warn_count,
        dirs_done: totals.dirs_done,
        dirs_missing: totals.dirs_missing,
        files_removed: totals.files_removed,
        kb_removed: totals.kb_removed,
        files_left: totals.files_left,
        measure_fail: totals.measure_fail,
        warnings: {
            let mut w = totals.warnings.clone();
            w.extend(repair_totals.warnings.clone());
            w
        },
        errors: {
            let mut e = totals.errors.clone();
            e.extend(repair_totals.errors.clone());
            e
        },
    };
    let _ = &mut totals;

    let repair_status = if !entered || (repair_results.is_empty() && repair_already_entered) {
        "not started a second time".to_string()
    } else if merged.err_count > err_at_repair_start {
        "ran - one or more operations reported a failure".to_string()
    } else {
        "ran - every operation completed without a reported failure".to_string()
    };

    AppSummary::compute(
        "completed".to_string(),
        repair_status,
        entered,
        merged,
        cleanup_results,
        repair_results,
        free_before,
        free_after,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{DirOutcome, ProgramOutcome};
    use std::path::Path;

    struct Mock;

    impl MaintenanceRunner for Mock {
        fn clean_dir(&mut self, _p: &Path) -> DirOutcome {
            DirOutcome {
                existed: true,
                files_removed: 1,
                kb_removed: 1,
                files_left: 0,
                measure_failed: false,
            }
        }
        fn run_program(&mut self, _p: &str, _a: &[&str]) -> ProgramOutcome {
            ProgramOutcome {
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
            }
        }
        fn program_exists(&mut self, _: &str) -> bool {
            false
        }
        fn emit(&mut self, _: ExecEvent) {}
    }

    #[test]
    fn all_runs_cleanup_before_repair() {
        let mut m = Mock;
        let summary = run_all_phases(&mut m, false);
        assert!(summary.repair_entered);
        assert!(!summary.cleanup_results.is_empty());
        assert_eq!(summary.repair_results.len(), 5);
        assert_eq!(summary.cleanup_status, "completed");
        assert_eq!(summary.exit_code, 0);
    }

    #[test]
    fn repair_failure_marks_overall_not_success() {
        struct FailRepair;
        impl MaintenanceRunner for FailRepair {
            fn clean_dir(&mut self, _p: &Path) -> DirOutcome {
                DirOutcome {
                    existed: true,
                    files_removed: 0,
                    kb_removed: 0,
                    files_left: 0,
                    measure_failed: false,
                }
            }
            fn run_program(&mut self, program: &str, _a: &[&str]) -> ProgramOutcome {
                let code = if program == "DISM.exe" { 1726 } else { 0 };
                ProgramOutcome {
                    exit_code: code,
                    stdout: String::new(),
                    stderr: String::new(),
                }
            }
            fn program_exists(&mut self, _: &str) -> bool {
                false
            }
            fn emit(&mut self, _: ExecEvent) {}
        }
        let mut m = FailRepair;
        let summary = run_all_phases(&mut m, false);
        assert!(summary.exit_code != 0);
        assert!(summary.repair_status.contains("failure"));
    }
}
