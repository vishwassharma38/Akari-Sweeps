//! Health/servicing phase — strict separation owner of all DISM work.
//!
//! Exact commands, in order:
//! 1. `DISM.exe /Online /Cleanup-Image /StartComponentCleanup`
//! 2. `DISM.exe /Online /Cleanup-Image /CheckHealth`
//! 3. `DISM.exe /Online /Cleanup-Image /ScanHealth`
//! 4. `DISM.exe /Online /Cleanup-Image /RestoreHealth`
//! 5. `sfc /scannow`
//! 6. `chkdsk %SystemDrive% /scan` (online scan; no reboot)
//!
//! `StartComponentCleanup` was historically BAT `[18/18]` cleanup; it
//! now runs here so `akariclean` launches zero DISM commands.

use crate::commands::clean::record_dism;
use crate::system::windows::system_drive;
use crate::system::{ExecEvent, MaintenanceRunner, OpResult, ProgressState, RunTotals};
use std::time::Instant;

/// Fixed six-step health workflow. Completed steps advance overall
/// progress; the running DISM/SFC/CHKDSK holds at the last defensible
/// percentage with a working marker — no fabricated intra-command
/// percentages and no log-line parsing as progress.
pub const HEALTH_STEP_COUNT: u64 = 6;

pub fn health_total_units() -> u64 {
    HEALTH_STEP_COUNT
}

fn emit_health_progress(
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

/// The six health steps in order: (id, banner, program, args).
/// `StartComponentCleanup` runs first so servicing cleanup precedes
/// diagnostics; remaining order preserves the BAT repair sequence.
pub fn repair_plan() -> Vec<(String, String, String, Vec<String>)> {
    vec![
        (
            "repair:compcleanup".to_string(),
            "[Repair 1/6] DISM - COMPONENT STORE CLEANUP".to_string(),
            "DISM.exe".to_string(),
            vec![
                "/Online".to_string(),
                "/Cleanup-Image".to_string(),
                "/StartComponentCleanup".to_string(),
            ],
        ),
        (
            "repair:checkhealth".to_string(),
            "[Repair 2/6] DISM - COMPONENT STORE HEALTH CHECK".to_string(),
            "DISM.exe".to_string(),
            vec![
                "/Online".to_string(),
                "/Cleanup-Image".to_string(),
                "/CheckHealth".to_string(),
            ],
        ),
        (
            "repair:scanhealth".to_string(),
            "[Repair 3/6] DISM - COMPONENT STORE DEEP SCAN".to_string(),
            "DISM.exe".to_string(),
            vec![
                "/Online".to_string(),
                "/Cleanup-Image".to_string(),
                "/ScanHealth".to_string(),
            ],
        ),
        (
            "repair:restorehealth".to_string(),
            "[Repair 4/6] DISM - COMPONENT STORE REPAIR".to_string(),
            "DISM.exe".to_string(),
            vec![
                "/Online".to_string(),
                "/Cleanup-Image".to_string(),
                "/RestoreHealth".to_string(),
            ],
        ),
        (
            "repair:sfc".to_string(),
            "[Repair 5/6] SYSTEM FILE CHECKER - SFC".to_string(),
            "sfc".to_string(),
            vec!["/scannow".to_string()],
        ),
        (
            "repair:chkdsk".to_string(),
            "[Repair 6/6] FILESYSTEM HEALTH CHECK".to_string(),
            "chkdsk".to_string(),
            vec![system_drive(), "/scan".to_string()],
        ),
    ]
}

/// Map an `sfc /scannow` exit code per the BAT: 0 = ok, else error.
pub fn record_sfc(totals: &mut RunTotals, exit_code: i32) -> OpResult {
    if exit_code == 0 {
        totals.add_ok();
        let mut r = OpResult::ok(
            "repair:sfc",
            "[Repair 5/6] SFC",
            "sfc /scannow",
            "SFC completed with exit code 0.".to_string(),
        );
        r.exit_code = Some(0);
        r
    } else {
        totals.add_error(format!(
            "SFC returned exit code {exit_code}; review its output and CBS.log."
        ));
        let mut r = OpResult::failed(
            "repair:sfc",
            "[Repair 5/6] SFC",
            "sfc /scannow",
            format!("SFC returned exit code {exit_code}."),
        );
        r.exit_code = Some(exit_code);
        r
    }
}

/// Map a `chkdsk /scan` exit code per the BAT:
/// 0 clean, 1 fixed, 2 informational, 3 incomplete (error), else unexpected.
pub fn record_chkdsk(totals: &mut RunTotals, exit_code: i32, drive: &str) -> OpResult {
    match exit_code {
        0 => {
            totals.add_ok();
            let mut r = OpResult::ok(
                "repair:chkdsk",
                "[Repair 6/6] CHKDSK",
                &format!("chkdsk {drive} /scan"),
                "CHKDSK online scan completed. No errors were found.".to_string(),
            );
            r.exit_code = Some(0);
            r
        }
        1 => {
            totals.add_ok();
            totals.add_warning(
                "CHKDSK reported that it found and fixed filesystem errors.".to_string(),
            );
            let mut r = OpResult::ok(
                "repair:chkdsk",
                "[Repair 6/6] CHKDSK",
                &format!("chkdsk {drive} /scan"),
                "CHKDSK completed. Errors were found and fixed.".to_string(),
            );
            r.exit_code = Some(1);
            r
        }
        2 => {
            totals.add_ok();
            let mut r = OpResult::ok(
                "repair:chkdsk",
                "[Repair 6/6] CHKDSK",
                &format!("chkdsk {drive} /scan"),
                "CHKDSK completed with informational exit code 2.".to_string(),
            );
            r.exit_code = Some(2);
            r
        }
        3 => {
            totals
                .add_error("CHKDSK could not complete or errors remain [exit code 3].".to_string());
            let mut r = OpResult::failed(
                "repair:chkdsk",
                "[Repair 6/6] CHKDSK",
                &format!("chkdsk {drive} /scan"),
                "CHKDSK returned exit code 3.".to_string(),
            );
            r.exit_code = Some(3);
            r
        }
        other => {
            totals.add_error(format!("CHKDSK returned unexpected exit code {other}."));
            let mut r = OpResult::failed(
                "repair:chkdsk",
                "[Repair 6/6] CHKDSK",
                &format!("chkdsk {drive} /scan"),
                format!("CHKDSK returned exit code {other}."),
            );
            r.exit_code = Some(other);
            r
        }
    }
}

/// Execute the repair phase once (BAT guards re-entry with `REPAIR_ENTERED`).
/// Returns (results, entered). When `already_entered`, warns and runs nothing.
///
/// The `RepairGate` proves the caller is authorized for the repair phase
/// (see `PhaseGate::for_command`). This function holds no cleanup gate,
/// so it cannot delete temporary files or invoke cleanup — isolation is
/// structural. It invokes only the six health commands
/// (DISM StartComponentCleanup, Check/Scan/RestoreHealth, SFC, CHKDSK).
pub fn run_repair_phase(
    runner: &mut dyn MaintenanceRunner,
    _gate: &crate::commands::RepairGate,
    already_entered: bool,
) -> (Vec<OpResult>, RunTotals, bool) {
    let mut totals = RunTotals::default();
    if already_entered {
        totals.add_warning(
            "The repair phase had already started in this run and was not started a second time."
                .to_string(),
        );
        return (Vec::new(), totals, true);
    }

    runner.emit(ExecEvent::PhaseStarted {
        name: "repair".to_string(),
    });
    let total = health_total_units();
    let phase_start = Instant::now();
    emit_health_progress(runner, 0, total, "Starting system repair…", false, false);
    runner.emit(ExecEvent::Log(
        "Windows will now check and repair system components and protected system files. This may take some time. Do not close this window.".to_string(),
    ));

    let mut results = Vec::new();
    let drive = system_drive();

    for (id, banner, program, args) in repair_plan() {
        runner.emit(ExecEvent::Log(banner.clone()));
        runner.emit(ExecEvent::OpStarted {
            id: id.clone(),
            label: banner.clone(),
            target: format!("{program} {}", args.join(" ")),
        });
        // Working state without advancing: the bar holds at the last
        // defensible percentage while this opaque utility runs (no
        // trustworthy intermediate progress exists for DISM/SFC/CHKDSK).
        emit_health_progress(
            runner,
            results.len() as u64,
            total,
            &format!("{banner} — working…"),
            totals.err_count > 0,
            true,
        );
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let t0 = Instant::now();
        let out = runner.run_program(&program, &arg_refs);
        let elapsed_ms = t0.elapsed().as_millis() as u64;
        // Stream a bounded tail for live output.
        for line in out
            .stdout
            .lines()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .iter()
            .rev()
        {
            runner.emit(ExecEvent::Log((*line).to_string()));
        }
        if !out.stderr.trim().is_empty() {
            for line in out.stderr.lines().take(4) {
                runner.emit(ExecEvent::Log(format!("stderr: {line}")));
            }
        }
        let mut result = if id == "repair:sfc" {
            record_sfc(&mut totals, out.exit_code)
        } else if id == "repair:chkdsk" {
            record_chkdsk(&mut totals, out.exit_code, &drive)
        } else {
            let op = match id.as_str() {
                "repair:compcleanup" => "StartComponentCleanup",
                "repair:checkhealth" => "CheckHealth",
                "repair:scanhealth" => "ScanHealth",
                _ => "RestoreHealth",
            };
            record_dism(&mut totals, op, out.exit_code, "")
        };
        result.duration_ms = Some(elapsed_ms);
        runner.emit(ExecEvent::OpFinished(result.clone()));
        results.push(result);
        // Evidence-based advance only (completed steps). Never parse log
        // lines as percentages; never invent intra-command progress.
        emit_health_progress(
            runner,
            results.len() as u64,
            total,
            &banner,
            totals.err_count > 0,
            false,
        );
    }

    let failed = totals.err_count > 0;
    let done_label = if failed {
        "Repair finished with failures"
    } else {
        "Repair complete"
    };
    emit_health_progress(
        runner,
        results.len() as u64,
        total,
        done_label,
        failed,
        false,
    );
    let phase_ms = phase_start.elapsed().as_millis() as u64;
    runner.emit(ExecEvent::Log(format!(
        "repair phase: {}/{} steps settled in {} ms (task-count approx; servicing utilities dominate)",
        results.len(),
        total,
        phase_ms
    )));

    runner.emit(ExecEvent::PhaseFinished {
        name: "repair".to_string(),
    });
    (results, totals, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{DirOutcome, MaintenanceRunner, OpStatus, ProgramOutcome};
    use std::collections::HashMap;
    use std::path::Path;

    struct Mock {
        codes: HashMap<String, i32>,
    }

    impl MaintenanceRunner for Mock {
        fn clean_dir(&mut self, _p: &Path) -> DirOutcome {
            DirOutcome {
                existed: false,
                files_removed: 0,
                kb_removed: 0,
                files_left: 0,
                measure_failed: false,
                locked_skipped: 0,
                permission_denied: 0,
                other_unremovable: 0,
            }
        }
        fn run_program(&mut self, program: &str, args: &[&str]) -> ProgramOutcome {
            let key = format!("{program} {}", args.join(" "));
            ProgramOutcome {
                exit_code: *self.codes.get(&key).unwrap_or(&0),
                stdout: String::new(),
                stderr: String::new(),
            }
        }
        fn program_exists(&mut self, _: &str) -> bool {
            true
        }
        fn emit(&mut self, _: ExecEvent) {}
    }

    #[test]
    fn plan_preserves_bat_sequence_and_args() {
        let plan = repair_plan();
        assert_eq!(plan.len(), 6);
        assert!(plan[0].3.contains(&"/StartComponentCleanup".to_string()));
        assert_eq!(plan[0].0, "repair:compcleanup");
        assert!(plan[1].3.contains(&"/CheckHealth".to_string()));
        assert!(plan[2].3.contains(&"/ScanHealth".to_string()));
        assert!(plan[3].3.contains(&"/RestoreHealth".to_string()));
        assert_eq!(plan[4].0, "repair:sfc");
        assert_eq!(plan[5].0, "repair:chkdsk");
        assert!(plan[5].3.contains(&"/scan".to_string()));
        // DISM prefix identical across the four DISM calls.
        for i in 0..4 {
            assert_eq!(plan[i].2, "DISM.exe");
            assert_eq!(plan[i].3[0], "/Online");
            assert_eq!(plan[i].3[1], "/Cleanup-Image");
        }
    }

    fn repair_gate() -> crate::commands::RepairGate {
        crate::commands::PhaseGate::for_command(crate::commands::AppCommand::Health)
            .1
            .unwrap()
    }

    #[test]
    fn repair_runs_once_guard() {
        let mut m = Mock {
            codes: HashMap::new(),
        };
        let (results, _, entered) = run_repair_phase(&mut m, &repair_gate(), true);
        assert!(entered);
        assert!(results.is_empty());
    }

    #[test]
    fn sfc_failure_is_error() {
        let mut t = RunTotals::default();
        let r = record_sfc(&mut t, 1);
        assert_eq!(r.status, OpStatus::Failed);
        assert_eq!(t.err_count, 1);
    }

    #[test]
    fn chkdsk_codes_match_bat() {
        for (code, ok, errs) in [
            (0, true, 0),
            (1, true, 0),
            (2, true, 0),
            (3, false, 1),
            (9, false, 1),
        ] {
            let mut t = RunTotals::default();
            let r = record_chkdsk(&mut t, code, "C:");
            assert_eq!(r.status == OpStatus::Success, ok, "code {code}");
            assert_eq!(t.err_count, errs, "code {code}");
        }
    }

    #[test]
    fn repair_all_ok_counts_six() {
        let mut m = Mock {
            codes: HashMap::new(),
        };
        let (results, totals, entered) = run_repair_phase(&mut m, &repair_gate(), false);
        assert!(entered);
        assert_eq!(results.len(), 6);
        assert_eq!(totals.err_count, 0);
        assert_eq!(totals.ok_count, 6);
    }

    // --- Requirement 3 regression tests: repair never cleans ---

    #[test]
    fn health_only_invokes_zero_cleaning_commands() {
        use crate::commands::runner::test_support::RecordingRunner;
        let mut r = RecordingRunner::new();
        let (results, totals, entered) = run_repair_phase(&mut r, &repair_gate(), false);
        assert!(entered);
        assert_eq!(results.len(), 6);
        assert!(
            r.cleanup_dir_calls().is_empty(),
            "akarihealth deleted files: {:?}",
            r.cleanup_dir_calls()
                .iter()
                .map(|c| c.dir.clone())
                .collect::<Vec<_>>()
        );
        assert!(
            r.cleanup_program_calls().is_empty(),
            "akarihealth ran cleanup programs: {:?}",
            r.cleanup_program_calls()
                .iter()
                .map(|c| format!("{} {}", c.program, c.args.join(" ")))
                .collect::<Vec<_>>()
        );
        assert_eq!(r.repair_calls().len(), 6);
        let _ = totals;
    }

    #[test]
    fn repair_error_paths_never_trigger_cleaning() {
        use crate::commands::runner::test_support::RecordingRunner;
        // Every repair command fails: error handling must stay in-phase.
        let mut r = RecordingRunner::failing(&[
            (
                "DISM.exe /Online /Cleanup-Image /StartComponentCleanup",
                1726,
            ),
            ("DISM.exe /Online /Cleanup-Image /CheckHealth", 1726),
            ("DISM.exe /Online /Cleanup-Image /ScanHealth", 1726),
            ("DISM.exe /Online /Cleanup-Image /RestoreHealth", 1726),
            ("sfc /scannow", 1),
        ]);
        let (results, totals, _) = run_repair_phase(&mut r, &repair_gate(), false);
        assert!(r.cleanup_dir_calls().is_empty());
        assert!(r.cleanup_program_calls().is_empty());
        // chkdsk uses the live system drive; only the five scripted
        // failures are asserted here via totals.
        assert!(totals.err_count >= 5, "failures must be recorded");
        assert_eq!(results.len(), 6);
    }

    #[test]
    fn dism_740_in_repair_is_elevation_failure_not_success() {
        use crate::commands::runner::test_support::RecordingRunner;
        let mut r = RecordingRunner::failing(&[
            (
                "DISM.exe /Online /Cleanup-Image /StartComponentCleanup",
                740,
            ),
            ("DISM.exe /Online /Cleanup-Image /CheckHealth", 740),
            ("DISM.exe /Online /Cleanup-Image /ScanHealth", 740),
            ("DISM.exe /Online /Cleanup-Image /RestoreHealth", 740),
            ("sfc /scannow", 0),
        ]);
        let (results, totals, _) = run_repair_phase(&mut r, &repair_gate(), false);
        assert_eq!(results.len(), 6);
        assert!(totals.err_count >= 4, "740s must be errors");
        for res in results.iter().filter(|x| x.id.starts_with("dism:")) {
            if res.exit_code == Some(740) {
                assert_eq!(res.status, OpStatus::Failed);
                assert!(res.detail.to_lowercase().contains("elevation"));
            }
        }
        assert!(
            totals.errors.iter().any(|e| e.contains("740")),
            "error log must identify the 740 operation"
        );
    }

    // --- Progress regression tests (prompt §4.2: health) ---

    struct ProgressSpy {
        inner: Mock,
        progress: Vec<crate::system::ProgressState>,
        finished: usize,
    }

    impl ProgressSpy {
        fn new() -> Self {
            Self {
                inner: Mock {
                    codes: HashMap::new(),
                },
                progress: Vec::new(),
                finished: 0,
            }
        }
    }

    impl crate::system::MaintenanceRunner for ProgressSpy {
        fn clean_dir(&mut self, p: &Path) -> crate::system::DirOutcome {
            self.inner.clean_dir(p)
        }
        fn run_program(&mut self, program: &str, args: &[&str]) -> crate::system::ProgramOutcome {
            self.inner.run_program(program, args)
        }
        fn program_exists(&mut self, p: &str) -> bool {
            self.inner.program_exists(p)
        }
        fn emit(&mut self, ev: crate::system::ExecEvent) {
            match ev {
                crate::system::ExecEvent::Progress(s) => self.progress.push(s),
                crate::system::ExecEvent::OpFinished(_) => self.finished += 1,
                _ => {}
            }
        }
    }

    #[test]
    fn health_progress_covers_all_six_without_fabrication() {
        let mut spy = ProgressSpy::new();
        let (results, _, _) = run_repair_phase(&mut spy, &repair_gate(), false);
        assert_eq!(results.len(), 6);
        assert_eq!(health_total_units(), 6);
        // Init (0/6) + working + advances + terminal: at least init and end.
        assert!(spy.progress.len() >= 2);
        let first = &spy.progress[0];
        assert_eq!((first.completed, first.total), (0, 6));
        let last = spy.progress.last().unwrap();
        assert_eq!((last.completed, last.total), (6, 6));
        assert!(!last.failed);
        // One OpFinished per step (evidence, never timer ticks).
        assert_eq!(spy.finished, 6);
        // Working snapshots hold at last defensible (completed < total)
        // with indeterminate set — never a mid-step fabricated advance.
        let working = spy.progress.iter().filter(|p| p.indeterminate).count();
        assert!(working >= 6, "each opaque step holds working state");
        for w in spy.progress.iter().filter(|p| p.indeterminate) {
            assert!(w.completed < w.total || w.total == 0);
        }
    }

    #[test]
    fn health_steps_record_durations() {
        let mut m = Mock {
            codes: HashMap::new(),
        };
        let (results, _, _) = run_repair_phase(&mut m, &repair_gate(), false);
        for r in &results {
            assert!(r.duration_ms.is_some(), "{} must carry timing", r.id);
        }
    }
}
