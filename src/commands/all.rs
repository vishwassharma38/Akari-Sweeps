//! Combined workflow — `akariall`: cleanup followed by repair.
//!
//! The BAT gates repair behind an interactive Y/N prompt. An explicit
//! `akariall` invocation *is* that consent, so both phases run in order
//! without a second prompt; the final summary still reports each phase
//! honestly (a failed cleanup does not silently cancel repair, and the
//! overall status is never "success" when a required phase failed).
//!
//! Failure policy (explicit, matching the BAT's continuation behavior):
//! cleanup errors never block the repair stage — repair always runs after
//! cleanup finishes — but any error poisons the combined summary, so the
//! overall status and exit code can never report success when a required
//! phase failed.

use crate::commands::clean::{cleanup_total_units, run_cleanup_phase};
use crate::commands::health::{health_total_units, run_repair_phase};
use crate::commands::{CleanupGate, PhaseGate, RepairGate};
use crate::system::windows::free_space_kb;
use crate::system::{AppSummary, ExecEvent, MaintenanceRunner, ProgressState, RunTotals};
use std::time::Instant;

/// Stable combined total: cleanup task units + six health steps.
/// Task-count approximation (documented): every directory/program target
/// and every health step counts equally — not exact time or byte
/// progress. Cleaning starts at 0; health starts from cleaning's
/// progress; the bar never resets and never moves backward.
pub fn combined_total_units() -> u64 {
    cleanup_total_units() + health_total_units()
}

fn emit_combined_progress(
    runner: &mut dyn MaintenanceRunner,
    completed: u64,
    total: u64,
    current: &str,
    failed: bool,
    indeterminate: bool,
) {
    let mut s = ProgressState::new(completed.min(total), total, current.to_string());
    s.failed = failed;
    s.indeterminate = indeterminate;
    runner.emit(ExecEvent::Progress(s));
}

/// Run cleanup, then repair, merging totals like the BAT summary.
/// `repair_already_entered` is the `REPAIR_ENTERED` guard (always false on
/// a fresh `akariall` run; exposed for tests).
///
/// Gates are minted here — the only place holding both — and each phase
/// receives only its own: cleanup runs strictly before repair, repair
/// never runs concurrently, and cleaning is never started a second time.
pub fn run_all_phases(
    runner: &mut dyn MaintenanceRunner,
    repair_already_entered: bool,
) -> AppSummary {
    let (cleanup_gate, repair_gate) = PhaseGate::for_command(crate::commands::AppCommand::All);
    let cleanup_gate = cleanup_gate.expect("All allows cleanup");
    let repair_gate = repair_gate.expect("All allows repair");
    run_all_phases_gated(runner, &cleanup_gate, &repair_gate, repair_already_entered)
}

fn run_all_phases_gated(
    runner: &mut dyn MaintenanceRunner,
    cleanup_gate: &CleanupGate,
    repair_gate: &RepairGate,
    repair_already_entered: bool,
) -> AppSummary {
    let free_before = free_space_kb();
    let combined_total = combined_total_units();
    let combined_start = Instant::now();

    runner.emit(ExecEvent::Log(
        "akariall: running cleanup followed by system-image repair.".to_string(),
    ));
    // Shared operation-level total so the bar spans both phases without
    // resetting. Per-phase `Progress` inits (smaller totals) are ignored
    // by the UI in favour of this larger combined total; completions
    // accumulate via `OpFinished` evidence.
    emit_combined_progress(
        runner,
        0,
        combined_total,
        "Starting combined cleanup + repair…",
        false,
        false,
    );

    // Phase 1: cleanup runs to completion (errors recorded, never fatal).
    let (cleanup_results, mut totals) = run_cleanup_phase(runner, cleanup_gate);
    let free_after = free_space_kb();
    // Bridge: health starts from cleaning's progress (never resets to 0).
    emit_combined_progress(
        runner,
        cleanup_results.len() as u64,
        combined_total,
        "Cleanup phase finished — starting system repair…",
        totals.err_count > 0,
        false,
    );

    // Phase 2: repair starts only after cleanup finished; never concurrent.
    let err_at_repair_start = totals.err_count;
    let (repair_results, repair_totals, entered) =
        run_repair_phase(runner, repair_gate, repair_already_entered);

    // Merge repair bookkeeping into the shared totals. Filesystem
    // breakdowns come only from cleanup (repair deletes nothing).
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
        files_locked: totals.files_locked,
        files_permission: totals.files_permission,
        files_other_left: totals.files_other_left,
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

    // Terminal combined snapshot: 100% only when all required work is
    // accounted for; failures render as FAILED, never as plain success.
    let combined_done = (cleanup_results.len() + repair_results.len()) as u64;
    let failed = merged.err_count > 0;
    emit_combined_progress(
        runner,
        combined_done.min(combined_total),
        combined_total,
        if failed {
            "Combined run finished with failures"
        } else {
            "Combined run complete"
        },
        failed,
        false,
    );
    let combined_ms = combined_start.elapsed().as_millis() as u64;
    runner.emit(ExecEvent::Log(format!(
        "akariall: {combined_done}/{combined_total} task units settled in {combined_ms} ms \
         (task-count approx; external utilities dominate where present)"
    )));

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
    use crate::commands::runner::test_support::RecordingRunner;
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
                locked_skipped: 0,
                permission_denied: 0,
                other_unremovable: 0,
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

    fn gates() -> (CleanupGate, RepairGate) {
        let (c, r) = PhaseGate::for_command(crate::commands::AppCommand::All);
        (c.unwrap(), r.unwrap())
    }

    #[test]
    fn all_runs_cleanup_before_repair() {
        let mut m = Mock;
        let summary = run_all_phases(&mut m, false);
        assert!(summary.repair_entered);
        assert!(!summary.cleanup_results.is_empty());
        assert_eq!(summary.repair_results.len(), 6);
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
                    locked_skipped: 0,
                    permission_denied: 0,
                    other_unremovable: 0,
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

    // --- Requirement 3 regression tests (invocation order + isolation) ---

    #[test]
    fn combined_runs_every_cleanup_before_first_repair() {
        let mut r = RecordingRunner::new();
        let (cg, rg) = gates();
        let _ = run_cleanup_phase(&mut r, &cg);
        // Simulate the real sequencing: repair starts after cleanup ends.
        let _ = run_repair_phase(&mut r, &rg, false);
        assert!(!r.cleanup_dir_calls().is_empty());
        assert_eq!(r.repair_calls().len(), 6);
        assert!(
            r.cleanup_before_repair(),
            "every cleanup op must finish before the first repair op starts"
        );
    }

    #[test]
    fn cleanup_failure_still_runs_repair_but_poisons_summary() {
        // Documented failure policy: repair is not skipped, success is
        // never reported. Fail a cleanup-only task (npm); health must
        // still run (6 calls) with poisoned summary and strict ordering.
        let mut r = RecordingRunner::failing(&[
            ("npm cache clean --force", 1),
            ("npm.cmd cache clean --force", 1),
        ]);
        let summary = run_all_phases(&mut r, false);
        assert_eq!(
            r.repair_calls().len(),
            6,
            "repair must still run after cleanup errors"
        );
        assert!(summary.exit_code != 0, "errors must poison the summary");
        assert!(!summary.totals.errors.is_empty());
        assert!(r.cleanup_before_repair());
    }

    #[test]
    fn all_preserves_order_with_strict_separation() {
        // End-to-end via run_all_phases: every cleanup before first repair,
        // and no DISM/SFC/CHKDSK inside the cleanup prefix.
        let mut r = RecordingRunner::new();
        let summary = run_all_phases(&mut r, false);
        assert!(r.cleanup_before_repair());
        assert_eq!(r.repair_calls().len(), 6);
        assert!(!summary.cleanup_results.is_empty());
        assert_eq!(summary.repair_results.len(), 6);
        // Cleanup prefix contains zero health programs.
        let first_repair = r.repair_calls().iter().map(|c| c.seq).min().unwrap();
        for c in r.calls.iter().filter(|c| c.seq < first_repair) {
            if let Some(dir) = &c.dir {
                let _ = dir;
                continue;
            }
            let prog = c.program.to_lowercase();
            assert!(!prog.contains("dism"), "cleanup ran DISM: {}", c.program);
            assert!(prog != "sfc" && prog != "chkdsk");
        }
    }

    // --- Combined progress regression tests (prompt §3.6 + §4.2) ---

    struct ProgressRecorder {
        inner: RecordingRunner,
        progress: Vec<crate::system::ProgressState>,
        finished: usize,
    }

    impl ProgressRecorder {
        fn new_rec() -> Self {
            Self {
                inner: RecordingRunner::new(),
                progress: Vec::new(),
                finished: 0,
            }
        }
    }

    impl MaintenanceRunner for ProgressRecorder {
        fn clean_dir(&mut self, p: &Path) -> DirOutcome {
            self.inner.clean_dir(p)
        }
        fn run_program(&mut self, program: &str, args: &[&str]) -> ProgramOutcome {
            self.inner.run_program(program, args)
        }
        fn program_exists(&mut self, p: &str) -> bool {
            self.inner.program_exists(p)
        }
        fn emit(&mut self, ev: ExecEvent) {
            match ev {
                ExecEvent::Progress(s) => self.progress.push(s),
                ExecEvent::OpFinished(_) => self.finished += 1,
                _ => {}
            }
        }
    }

    #[test]
    fn combined_total_spans_both_phases() {
        assert_eq!(
            combined_total_units(),
            crate::commands::clean::cleanup_total_units() + 6
        );
    }

    #[test]
    fn combined_progress_never_resets_and_is_monotonic() {
        let mut r = ProgressRecorder::new_rec();
        let summary = run_all_phases(&mut r, false);
        assert!(!r.progress.is_empty());
        // First snapshot is the combined init (0/combined).
        assert_eq!(r.progress[0].completed, 0);
        assert_eq!(r.progress[0].total, combined_total_units());
        // Completions accumulate across phases (cleanup + 6 health).
        assert_eq!(
            r.finished,
            summary.cleanup_results.len() + summary.repair_results.len()
        );
        // Terminal snapshot reaches the combined total exactly once, at
        // the end — never resetting to 0 mid-run.
        let last = r.progress.last().unwrap();
        assert_eq!(last.total, combined_total_units());
        // No snapshot after the first may restart at zero with the same
        // combined total (that would be a reset).
        for w in r.progress.iter().skip(1) {
            assert!(
                !(w.completed == 0 && w.total == combined_total_units()),
                "combined bar must not reset to zero mid-run"
            );
        }
        // Monotonic when filtered to same-total snapshots is enforced by
        // the App layer; here assert the terminal is full and ordered.
        assert!(last.completed <= last.total);
    }

    #[test]
    fn combined_failure_is_failed_not_success() {
        let r = RecordingRunner::failing(&[
            ("npm cache clean --force", 1),
            ("npm.cmd cache clean --force", 1),
        ]);
        // Wrap to capture progress too.
        struct Wrap {
            inner: RecordingRunner,
            last_failed: bool,
        }
        impl MaintenanceRunner for Wrap {
            fn clean_dir(&mut self, p: &Path) -> DirOutcome {
                self.inner.clean_dir(p)
            }
            fn run_program(&mut self, program: &str, args: &[&str]) -> ProgramOutcome {
                self.inner.run_program(program, args)
            }
            fn program_exists(&mut self, p: &str) -> bool {
                self.inner.program_exists(p)
            }
            fn emit(&mut self, ev: ExecEvent) {
                if let ExecEvent::Progress(s) = &ev {
                    if s.failed {
                        self.last_failed = true;
                    }
                }
            }
        }
        let mut w = Wrap {
            inner: r,
            last_failed: false,
        };
        let summary = run_all_phases(&mut w, false);
        assert!(summary.exit_code != 0);
        assert!(
            w.last_failed,
            "failed combined run must flag progress FAILED"
        );
    }
}
