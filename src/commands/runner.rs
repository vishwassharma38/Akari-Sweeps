//! Real side-effect runner: filesystem + subprocesses + event fan-out.

use crate::commands::clean::{clean_dir_contents, measure_dir};
use crate::system::process::{program_exists, run_program_capture};
use crate::system::{DirOutcome, EventTx, ExecEvent, MaintenanceRunner, ProgramOutcome};
use std::path::Path;

/// Production runner. `clean_dir` mirrors BAT `:CleanDir`/`:TallyDir`;
/// locked files are skipped, measurements are best-effort.
pub struct RealRunner {
    tx: Option<EventTx>,
}

impl RealRunner {
    pub fn new(tx: Option<EventTx>) -> Self {
        Self { tx }
    }
}

impl MaintenanceRunner for RealRunner {
    fn clean_dir(&mut self, path: &Path) -> DirOutcome {
        if !path.exists() {
            return DirOutcome {
                existed: false,
                files_removed: 0,
                kb_removed: 0,
                files_left: 0,
                measure_failed: false,
            };
        }
        let before = measure_dir(path);
        clean_dir_contents(path);
        let after = measure_dir(path);
        match (before, after) {
            (Some((b_items, b_kb)), Some((a_items, _a_kb))) => {
                let removed = b_items.saturating_sub(a_items);
                // KB remeasured from scratch (BAT diffs KB the same way).
                let after_kb = measure_dir(path).map(|(_, kb)| kb).unwrap_or(0);
                let kb_removed = b_kb.saturating_sub(after_kb);
                DirOutcome {
                    existed: true,
                    files_removed: removed,
                    kb_removed,
                    files_left: a_items,
                    measure_failed: false,
                }
            }
            _ => DirOutcome {
                existed: true,
                files_removed: 0,
                kb_removed: 0,
                files_left: 0,
                measure_failed: true,
            },
        }
    }

    fn run_program(&mut self, program: &str, args: &[&str]) -> ProgramOutcome {
        run_program_capture(program, args)
    }

    fn program_exists(&mut self, program: &str) -> bool {
        program_exists(program)
    }

    fn emit(&mut self, event: ExecEvent) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(event);
        }
    }
}

/// Operation classification for isolation tests.
///
/// Repair commands are exactly the five `[Repair 1/5]`–`[5/5]` operations:
/// `DISM /CheckHealth`, `DISM /ScanHealth`, `DISM /RestoreHealth`,
/// `sfc /scannow`, `chkdsk … /scan`.
///
/// `DISM /StartComponentCleanup` is **not** repair: the BAT runs it as
/// `[18/18]` inside the cleanup phase ("Windows component-store
/// cleanup", listed under CLEANS), so it classifies as cleanup.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
pub enum CallKind {
    CleanDir,
    CleanupProgram,
    RepairProgram,
}

#[cfg(test)]
impl CallKind {
    pub fn classify_program(program: &str, args: &[&str]) -> Self {
        let p = program.to_lowercase();
        let joined = args.join(" ").to_lowercase();
        let is_dism = p.contains("dism");
        if (is_dism
            && (joined.contains("checkhealth")
                || joined.contains("scanhealth")
                || joined.contains("restorehealth")))
            || p == "sfc"
            || p == "chkdsk"
        {
            CallKind::RepairProgram
        } else {
            CallKind::CleanupProgram
        }
    }
}

/// Deterministic recording runner for isolation regression tests.
/// Logs every side effect in order; never touches the real filesystem
/// or real Windows servicing commands.
#[cfg(test)]
pub mod test_support {
    use super::CallKind;
    use crate::system::{DirOutcome, ExecEvent, MaintenanceRunner, ProgramOutcome};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    #[derive(Debug, Clone)]
    pub struct RecordedCall {
        pub seq: u64,
        pub kind: CallKind,
        pub program: String,
        pub args: Vec<String>,
        pub dir: Option<PathBuf>,
    }

    pub struct RecordingRunner {
        seq: u64,
        pub calls: Vec<RecordedCall>,
        /// Program exit codes keyed by `"program arg1 arg2"`. Default 0.
        pub exits: HashMap<String, i32>,
        /// Programs reported as installed. Default: all present.
        pub installed: Option<Vec<String>>,
    }

    impl RecordingRunner {
        pub fn new() -> Self {
            Self {
                seq: 0,
                calls: Vec::new(),
                exits: HashMap::new(),
                installed: None,
            }
        }

        pub fn failing(programs: &[(&str, i32)]) -> Self {
            let mut r = Self::new();
            for (key, code) in programs {
                r.exits.insert((*key).to_string(), *code);
            }
            r
        }

        fn record(&mut self, kind: CallKind, program: &str, args: &[&str], dir: Option<PathBuf>) {
            self.seq += 1;
            self.calls.push(RecordedCall {
                seq: self.seq,
                kind,
                program: program.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
                dir,
            });
        }

        pub fn repair_calls(&self) -> Vec<&RecordedCall> {
            self.calls
                .iter()
                .filter(|c| c.kind == CallKind::RepairProgram)
                .collect()
        }

        pub fn cleanup_dir_calls(&self) -> Vec<&RecordedCall> {
            self.calls
                .iter()
                .filter(|c| c.kind == CallKind::CleanDir)
                .collect()
        }

        pub fn cleanup_program_calls(&self) -> Vec<&RecordedCall> {
            self.calls
                .iter()
                .filter(|c| c.kind == CallKind::CleanupProgram)
                .collect()
        }

        /// True when every cleanup side effect finished before the first
        /// repair side effect started (strict `akariall` ordering).
        pub fn cleanup_before_repair(&self) -> bool {
            let last_cleanup = self
                .calls
                .iter()
                .filter(|c| c.kind != CallKind::RepairProgram)
                .map(|c| c.seq)
                .max();
            let first_repair = self.repair_calls().iter().map(|c| c.seq).min();
            match (last_cleanup, first_repair) {
                (Some(l), Some(f)) => l < f,
                _ => false,
            }
        }
    }

    impl MaintenanceRunner for RecordingRunner {
        fn clean_dir(&mut self, path: &Path) -> DirOutcome {
            self.record(
                CallKind::CleanDir,
                "clean_dir",
                &[],
                Some(path.to_path_buf()),
            );
            DirOutcome {
                existed: true,
                files_removed: 4,
                kb_removed: 16,
                files_left: 0,
                measure_failed: false,
            }
        }

        fn run_program(&mut self, program: &str, args: &[&str]) -> ProgramOutcome {
            let kind = CallKind::classify_program(program, args);
            self.record(kind, program, args, None);
            let key = format!("{program} {}", args.join(" "));
            ProgramOutcome {
                exit_code: *self.exits.get(&key).unwrap_or(&0),
                stdout: String::new(),
                stderr: String::new(),
            }
        }

        fn program_exists(&mut self, program: &str) -> bool {
            match &self.installed {
                Some(list) => list.iter().any(|p| p == program),
                None => true,
            }
        }

        fn emit(&mut self, _event: ExecEvent) {}
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn classifier_separates_repair_from_cleanup() {
            assert_eq!(
                CallKind::classify_program("DISM.exe", &["/Online", "/CheckHealth"]),
                CallKind::RepairProgram
            );
            assert_eq!(
                CallKind::classify_program("sfc", &["/scannow"]),
                CallKind::RepairProgram
            );
            assert_eq!(
                CallKind::classify_program("chkdsk", &["C:", "/scan"]),
                CallKind::RepairProgram
            );
            // Component-store cleanup belongs to the cleanup phase.
            assert_eq!(
                CallKind::classify_program(
                    "DISM.exe",
                    &["/Online", "/Cleanup-Image", "/StartComponentCleanup"]
                ),
                CallKind::CleanupProgram
            );
            assert_eq!(
                CallKind::classify_program("cleanmgr.exe", &["/verylowdisk"]),
                CallKind::CleanupProgram
            );
        }
    }
}
