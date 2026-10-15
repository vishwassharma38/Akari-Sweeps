//! Repair phase — faithful port of BAT `:PhaseRepair` ([Repair 1/5]–[5/5]).
//!
//! Exact commands, in order:
//! 1. `DISM.exe /Online /Cleanup-Image /CheckHealth`
//! 2. `DISM.exe /Online /Cleanup-Image /ScanHealth`
//! 3. `DISM.exe /Online /Cleanup-Image /RestoreHealth`
//! 4. `sfc /scannow`
//! 5. `chkdsk %SystemDrive% /scan` (online scan; no reboot)

use crate::commands::clean::record_dism;
use crate::system::windows::system_drive;
use crate::system::{ExecEvent, MaintenanceRunner, OpResult, RunTotals};

/// The five repair steps in BAT order: (id, banner, program, args).
pub fn repair_plan() -> Vec<(String, String, String, Vec<String>)> {
    vec![
        (
            "repair:checkhealth".to_string(),
            "[Repair 1/5] DISM - COMPONENT STORE HEALTH CHECK".to_string(),
            "DISM.exe".to_string(),
            vec![
                "/Online".to_string(),
                "/Cleanup-Image".to_string(),
                "/CheckHealth".to_string(),
            ],
        ),
        (
            "repair:scanhealth".to_string(),
            "[Repair 2/5] DISM - COMPONENT STORE DEEP SCAN".to_string(),
            "DISM.exe".to_string(),
            vec![
                "/Online".to_string(),
                "/Cleanup-Image".to_string(),
                "/ScanHealth".to_string(),
            ],
        ),
        (
            "repair:restorehealth".to_string(),
            "[Repair 3/5] DISM - COMPONENT STORE REPAIR".to_string(),
            "DISM.exe".to_string(),
            vec![
                "/Online".to_string(),
                "/Cleanup-Image".to_string(),
                "/RestoreHealth".to_string(),
            ],
        ),
        (
            "repair:sfc".to_string(),
            "[Repair 4/5] SYSTEM FILE CHECKER - SFC".to_string(),
            "sfc".to_string(),
            vec!["/scannow".to_string()],
        ),
        (
            "repair:chkdsk".to_string(),
            "[Repair 5/5] FILESYSTEM HEALTH CHECK".to_string(),
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
            "[Repair 4/5] SFC",
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
            "[Repair 4/5] SFC",
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
                "[Repair 5/5] CHKDSK",
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
                "[Repair 5/5] CHKDSK",
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
                "[Repair 5/5] CHKDSK",
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
                "[Repair 5/5] CHKDSK",
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
                "[Repair 5/5] CHKDSK",
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
pub fn run_repair_phase(
    runner: &mut dyn MaintenanceRunner,
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
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let out = runner.run_program(&program, &arg_refs);
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
        let result = if id == "repair:sfc" {
            record_sfc(&mut totals, out.exit_code)
        } else if id == "repair:chkdsk" {
            record_chkdsk(&mut totals, out.exit_code, &drive)
        } else {
            let op = match id.as_str() {
                "repair:checkhealth" => "CheckHealth",
                "repair:scanhealth" => "ScanHealth",
                _ => "RestoreHealth",
            };
            record_dism(&mut totals, op, out.exit_code, "")
        };
        runner.emit(ExecEvent::OpFinished(result.clone()));
        results.push(result);
    }

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
        assert_eq!(plan.len(), 5);
        assert!(plan[0].3.contains(&"/CheckHealth".to_string()));
        assert!(plan[1].3.contains(&"/ScanHealth".to_string()));
        assert!(plan[2].3.contains(&"/RestoreHealth".to_string()));
        assert_eq!(plan[3].0, "repair:sfc");
        assert_eq!(plan[4].0, "repair:chkdsk");
        assert!(plan[4].3.contains(&"/scan".to_string()));
        // DISM prefix identical across the three DISM calls.
        for i in 0..3 {
            assert_eq!(plan[i].2, "DISM.exe");
            assert_eq!(plan[i].3[0], "/Online");
            assert_eq!(plan[i].3[1], "/Cleanup-Image");
        }
    }

    #[test]
    fn repair_runs_once_guard() {
        let mut m = Mock {
            codes: HashMap::new(),
        };
        let (results, _, entered) = run_repair_phase(&mut m, true);
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
    fn repair_all_ok_counts_five() {
        let mut m = Mock {
            codes: HashMap::new(),
        };
        let (results, totals, entered) = run_repair_phase(&mut m, false);
        assert!(entered);
        assert_eq!(results.len(), 5);
        assert_eq!(totals.err_count, 0);
        assert_eq!(totals.ok_count, 5);
    }
}
