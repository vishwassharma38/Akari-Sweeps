//! Application state, screen navigation, and worker-event handling.

use crate::akari::animation::{AnimationMode, AnimationSet, Animator};
use crate::akari::sprites::SpriteFrame;
use crate::commands::AppCommand;
use crate::system::{AppSummary, ExecEvent, OpResult, OpStatus, ProgressState};
use std::sync::mpsc::Receiver;
use std::time::Instant;

pub const PRODUCT_LABEL: &str = "Akari-Sweeps_V00.1";
pub const PRODUCT_VERSION: &str = "V00.1";
pub const LOG_CAP: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Maintenance,
    About,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionState {
    Idle,
    Running,
    Done,
    Error,
}

/// Loaded frames plus a diagnostic for one animation set.
#[derive(Debug)]
pub struct SpriteFrames {
    pub frames: Vec<SpriteFrame>,
    pub error: Option<String>,
}

impl SpriteFrames {
    fn load(set: AnimationSet) -> Self {
        match set.sprite_set().load_frames_best_effort() {
            Ok(frames) => Self {
                frames,
                error: None,
            },
            Err(error) => Self {
                frames: Vec::new(),
                error: Some(error),
            },
        }
    }
}

/// One entry per implemented `AnimationSet`, loaded through the shared
/// sprite pipeline so selection only swaps which cached set is rendered.
#[derive(Debug)]
pub struct SpriteLibrary {
    sets: [SpriteFrames; AnimationSet::COUNT],
}

impl SpriteLibrary {
    pub fn load_all() -> Self {
        Self {
            sets: AnimationSet::ALL.map(SpriteFrames::load),
        }
    }

    pub fn get(&self, set: AnimationSet) -> &SpriteFrames {
        &self.sets[set.index()]
    }

    #[cfg(test)]
    pub fn get_mut(&mut self, set: AnimationSet) -> &mut SpriteFrames {
        &mut self.sets[set.index()]
    }
}

pub struct App {
    pub command: AppCommand,
    pub screen: Screen,
    pub execution: ExecutionState,
    pub log: Vec<String>,
    pub operations: Vec<OpItem>,
    /// Evidence-based operation progress (single coherent model for
    /// clean/health/combined). Total comes from the worker's `Progress`
    /// init (stable per run); completions advance on `OpFinished`
    /// evidence; the running task shows as working without advancing.
    pub progress: ProgressState,
    pub animator: Animator,
    pub sprites: SpriteLibrary,
    pub summary: Option<AppSummary>,
    pub exit_code: i32,
    pub started: Instant,
    pub worker_running: bool,
    pub status_line: String,
    /// Guard against double-start from repeated keypresses/events.
    pub run_token: Option<u64>,
    pub scroll: usize,
    /// Command-entry field text (visible on every screen).
    pub input: String,
    /// When true, keystrokes edit `input`; shortcuts are suspended
    /// except Enter (submit), Esc (cancel), Backspace, and Ctrl+U (clear).
    pub input_focused: bool,
    /// Administrator-elevation state captured at startup.
    pub elevated: bool,
}

#[derive(Debug, Clone)]
pub struct OpItem {
    pub id: String,
    pub label: String,
    pub target: String,
    pub status: OpStatus,
    pub detail: String,
}

impl App {
    pub fn new(command: AppCommand) -> Self {
        let screen = match command {
            AppCommand::About => Screen::About,
            AppCommand::Help => Screen::Help,
            AppCommand::Clean | AppCommand::Health | AppCommand::All => Screen::Maintenance,
            AppCommand::Home => Screen::Home,
        };
        // Maintenance aliases auto-start; Home waits for a key.
        let execution = match command {
            AppCommand::Clean | AppCommand::Health | AppCommand::All => ExecutionState::Running,
            AppCommand::Home => ExecutionState::Idle,
            AppCommand::About | AppCommand::Help => ExecutionState::Idle,
        };
        let mut app = Self {
            command,
            screen,
            execution,
            log: vec![format!(
                "{PRODUCT_LABEL} — type a command (e.g. akarihelp for help)."
            )],
            operations: Vec::new(),
            progress: ProgressState::default(),
            animator: Animator::new(AnimationSet::Sweep),
            sprites: SpriteLibrary::load_all(),
            summary: None,
            exit_code: 0,
            started: Instant::now(),
            worker_running: false,
            status_line: String::new(),
            run_token: None,
            scroll: 0,
            input: String::new(),
            input_focused: false,
            elevated: crate::system::windows::is_elevated(),
        };
        app.sync_animation();
        app
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// Frames belonging to the currently selected animation set.
    pub fn frames(&self) -> &[SpriteFrame] {
        &self.sprites.get(self.animator.set()).frames
    }

    /// Load diagnostic for the currently selected animation set.
    pub fn sprite_error(&self) -> Option<&str> {
        self.sprites.get(self.animator.set()).error.as_deref()
    }

    /// Keep the animation honest: idle reading on Home, sweep only while
    /// maintenance is involved, settle when work ends.
    pub fn sync_animation(&mut self) {
        let (set, mode) = match self.execution {
            ExecutionState::Running => (AnimationSet::Sweep, AnimationMode::Sweeping),
            ExecutionState::Done | ExecutionState::Error => {
                (AnimationSet::Sweep, AnimationMode::Settled)
            }
            ExecutionState::Idle => match self.screen {
                Screen::Home => (AnimationSet::Idle, AnimationMode::Ambient),
                _ => (AnimationSet::Sweep, AnimationMode::Ambient),
            },
        };
        let now_ms = self.elapsed_ms();
        self.animator.select_set(set, now_ms);
        self.animator.set_mode(mode);
    }

    pub fn push_log(&mut self, line: String) {
        // Split embedded newlines so the panel never corrupts on long output.
        for part in line.split('\n') {
            let clean = part.trim_end().to_string();
            if clean.is_empty() {
                continue;
            }
            // Truncate pathological single lines.
            let shown = if clean.len() > 300 {
                format!("{}…", &clean[..300])
            } else {
                clean
            };
            self.log.push(shown);
            if self.log.len() > LOG_CAP {
                self.log.drain(0..self.log.len() - LOG_CAP);
            }
        }
    }

    fn upsert_op(&mut self, id: &str, label: &str, target: &str, status: OpStatus) {
        if let Some(op) = self.operations.iter_mut().find(|o| o.id == id) {
            op.status = status;
            op.label = label.to_string();
            op.target = target.to_string();
        } else {
            self.operations.push(OpItem {
                id: id.to_string(),
                label: label.to_string(),
                target: target.to_string(),
                status,
                detail: String::new(),
            });
        }
    }

    /// Handle one worker event. Returns true when a full summary arrived.
    pub fn on_event(&mut self, ev: ExecEvent) -> bool {
        match ev {
            ExecEvent::Log(line) => {
                self.push_log(line);
                false
            }
            ExecEvent::OpStarted { id, label, target } => {
                self.upsert_op(&id, &label, &target, OpStatus::Running);
                // Working state: hold at the last defensible percentage and
                // name the current task (no fabricated advance).
                if self.progress.total > 0 {
                    self.progress.current = format!("{label} — {target}");
                    self.progress.indeterminate = true;
                }
                self.push_log(format!("▶ {label} — {target}"));
                false
            }
            ExecEvent::OpProgress { id, line } => {
                self.push_log(format!("  [{id}] {line}"));
                false
            }
            ExecEvent::OpFinished(r) => {
                self.apply_result(&r);
                false
            }
            ExecEvent::PhaseStarted { name } => {
                self.push_log(format!("━━ {name} started ━━"));
                false
            }
            ExecEvent::PhaseFinished { name } => {
                self.push_log(format!("━━ {name} finished ━━"));
                false
            }
            ExecEvent::Progress(p) => {
                self.apply_progress(p);
                false
            }
            ExecEvent::Finished(summary) => {
                self.apply_summary(summary);
                true
            }
            ExecEvent::Error(e) => {
                self.execution = ExecutionState::Error;
                self.progress.failed = true;
                self.progress.indeterminate = false;
                self.push_log(format!("ERROR: {e}"));
                self.status_line = e;
                self.sync_animation();
                false
            }
        }
    }

    /// Merge a worker progress snapshot. Single coherent model:
    /// - First init (or larger combined total) establishes the stable
    ///   denominator; smaller per-phase inits during a combined run are
    ///   ignored for the total so the bar never resets.
    /// - Completions advance only via `OpFinished` evidence (see
    ///   `apply_result`); snapshots carry totals and working states, never
    ///   timer-based percentages.
    /// - Failure is sticky: once any task fails, the bar stays FAILED.
    fn apply_progress(&mut self, p: ProgressState) {
        if self.progress.total == 0 {
            self.progress = p;
            return;
        }
        if p.total > self.progress.total {
            // Larger combined total arriving (e.g. akariall init after a
            // phase init): keep accumulated completions, adopt the total.
            let completed = self.progress.completed;
            let failed = self.progress.failed || p.failed;
            self.progress = p;
            self.progress.completed = completed.min(self.progress.total);
            self.progress.failed = failed;
            return;
        }
        if p.total < self.progress.total {
            // Smaller per-phase init/update during a combined run: keep the
            // combined total so progress never resets or moves backward.
            // Adopt working-state details without touching the total.
            if !p.current.is_empty() {
                self.progress.current = p.current;
            }
            self.progress.failed = self.progress.failed || p.failed;
            // Only adopt indeterminate when it names real work; terminal
            // per-phase "complete" messages must not clear the combined
            // working state prematurely (completions drive that).
            if p.indeterminate {
                self.progress.indeterminate = true;
            }
            return;
        }
        // Same total: monotonic completions only (never backward), sticky
        // failure, latest working state.
        if p.completed > self.progress.completed {
            self.progress.completed = p.completed.min(self.progress.total);
        }
        if !p.current.is_empty() {
            self.progress.current = p.current;
        }
        self.progress.failed = self.progress.failed || p.failed;
        self.progress.indeterminate = p.indeterminate;
        if p.cancelled {
            self.progress.cancelled = true;
        }
    }

    fn apply_result(&mut self, r: &OpResult) {
        self.upsert_op(&r.id, &r.label, &r.target, r.status);
        if let Some(op) = self.operations.iter_mut().find(|o| o.id == r.id) {
            op.detail = r.detail.clone();
        }
        // Evidence-based advance: one completed work unit per settled
        // operation (success, warning, failure, or skip all count — they
        // all represent attempted work). Clamped to the stable total so a
        // filesystem race can never push past 100%; monotonic by
        // construction since completions only increment here.
        if self.progress.total > 0 {
            self.progress.completed = (self.progress.completed + 1).min(self.progress.total);
            self.progress.indeterminate = false;
            self.progress.current = format!("{} — {}", r.label, r.detail);
            if r.status == OpStatus::Failed {
                self.progress.failed = true;
            }
        } else if r.status == OpStatus::Failed {
            self.progress.failed = true;
        }
        let mark = match r.status {
            OpStatus::Success => "✓",
            OpStatus::Failed => "✗",
            OpStatus::Skipped => "–",
            _ => "…",
        };
        self.push_log(format!("{} {} — {}", mark, r.label, r.detail));
    }

    fn apply_summary(&mut self, summary: AppSummary) {
        self.exit_code = summary.exit_code;
        // Merge per-op results that arrived only in the summary.
        for r in summary
            .cleanup_results
            .iter()
            .chain(summary.repair_results.iter())
        {
            self.upsert_op(&r.id, &r.label, &r.target, r.status);
            if let Some(op) = self.operations.iter_mut().find(|o| o.id == r.id) {
                op.detail = r.detail.clone();
            }
        }
        // Terminal progress: 100% only when all required work is accounted
        // for. Failures render as FAILED (never plain success); the bar
        // never claims completion early and never moves backward here.
        if self.progress.total > 0 {
            self.progress.completed = self.progress.total;
            self.progress.indeterminate = false;
            if summary.totals.err_count > 0 {
                self.progress.failed = true;
            }
            self.progress.current = summary.overall.clone();
        } else if summary.totals.err_count > 0 {
            self.progress.failed = true;
        }
        self.push_log(format!("━━ summary: {} ━━", summary.overall));
        for w in &summary.totals.warnings {
            self.push_log(format!("warning: {w}"));
        }
        for e in &summary.totals.errors {
            self.push_log(format!("error: {e}"));
        }
        self.summary = Some(summary);
        self.execution = ExecutionState::Done;
        self.worker_running = false;
        self.sync_animation();
    }

    /// Drain all pending worker events without blocking the TUI.
    /// Returns the number drained (drives redraw scheduling).
    pub fn drain_events(&mut self, rx: &Receiver<ExecEvent>) -> usize {
        let mut n = 0;
        while let Ok(ev) = rx.try_recv() {
            self.on_event(ev);
            n += 1;
        }
        n
    }

    pub fn mark_started(&mut self, token: u64) {
        self.run_token = Some(token);
        self.worker_running = true;
        self.execution = ExecutionState::Running;
        self.operations.clear();
        self.progress = ProgressState::default();
        self.summary = None;
        self.sync_animation();
    }

    /// Focus (`/` key) or unfocus (Esc) the command-entry field.
    /// Focusing never starts an operation.
    pub fn set_input_focus(&mut self, focused: bool) {
        self.input_focused = focused;
        if focused {
            self.push_log(
                "Command entry: type a command, Enter to run, Esc to cancel.".to_string(),
            );
        }
    }

    /// Append typed text to the entry field (called only while focused).
    pub fn push_input(&mut self, text: &str) {
        const MAX: usize = 64;
        for ch in text.chars() {
            if self.input.len() >= MAX {
                break;
            }
            // Printable characters only; control keys are handled by the caller.
            if !ch.is_control() {
                self.input.push(ch);
            }
        }
    }

    pub fn backspace_input(&mut self) {
        self.input.pop();
    }

    pub fn clear_input(&mut self) {
        self.input.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::RunTotals;

    #[test]
    fn home_starts_idle_without_maintenance() {
        let app = App::new(AppCommand::Home);
        assert_eq!(app.screen, Screen::Home);
        assert_eq!(app.execution, ExecutionState::Idle);
        assert_eq!(app.animator.set(), AnimationSet::Idle);
        assert_eq!(app.animator.mode(), AnimationMode::Ambient);
        assert!(app.summary.is_none());
    }

    #[test]
    fn home_selects_idle_while_waiting_for_input() {
        let mut app = App::new(AppCommand::Home);
        app.set_input_focus(true);
        app.push_input("akari");
        app.sync_animation();
        assert_eq!(app.animator.set(), AnimationSet::Idle);
        assert_eq!(app.animator.mode(), AnimationMode::Ambient);
    }

    #[test]
    fn selected_set_resolves_to_matching_loaded_frames() {
        for (cmd, expected) in [
            (AppCommand::Home, AnimationSet::Idle),
            (AppCommand::Clean, AnimationSet::Sweep),
        ] {
            let app = App::new(cmd);
            assert_eq!(app.animator.set(), expected);
            if app.frames().is_empty() {
                eprintln!("skipping: {} assets not in tree", expected.sprite_set().dir);
                continue;
            }
            let spec = expected.sprite_set();
            assert_eq!(app.frames().len(), spec.frame_count);
            assert_eq!(
                (app.frames()[0].width, app.frames()[0].height),
                (spec.width, spec.height)
            );
        }
    }

    #[test]
    fn maintenance_aliases_enter_running_screen() {
        for cmd in [AppCommand::Clean, AppCommand::Health, AppCommand::All] {
            let app = App::new(cmd);
            assert_eq!(app.screen, Screen::Maintenance);
            assert_eq!(app.execution, ExecutionState::Running);
            assert_eq!(app.animator.set(), AnimationSet::Sweep);
            assert_eq!(app.animator.mode(), AnimationMode::Sweeping);
        }
    }

    #[test]
    fn about_help_do_not_run_maintenance() {
        for cmd in [AppCommand::About, AppCommand::Help] {
            let app = App::new(cmd);
            assert_eq!(app.execution, ExecutionState::Idle);
            assert!(app.summary.is_none());
            // Preserve their current ambient sweep behavior.
            assert_eq!(app.animator.set(), AnimationSet::Sweep);
            assert_eq!(app.animator.mode(), AnimationMode::Ambient);
        }
    }

    #[test]
    fn failed_op_reported_and_animation_settles() {
        let mut app = App::new(AppCommand::Clean);
        app.on_event(ExecEvent::OpStarted {
            id: "clean:npm".into(),
            label: "npm".into(),
            target: "npm cache".into(),
        });
        let mut r = OpResult::failed("clean:npm", "npm", "npm cache", "exit 1".into());
        r.exit_code = Some(1);
        app.on_event(ExecEvent::OpFinished(r));
        let op = app.operations.iter().find(|o| o.id == "clean:npm").unwrap();
        assert_eq!(op.status, OpStatus::Failed);

        let totals = RunTotals::default();
        let summary = AppSummary::compute(
            "completed".into(),
            "skipped".into(),
            false,
            totals,
            vec![],
            vec![],
            None,
            None,
        );
        app.on_event(ExecEvent::Finished(summary));
        assert_eq!(app.execution, ExecutionState::Done);
        assert_eq!(app.animator.set(), AnimationSet::Sweep);
        assert_eq!(app.animator.mode(), AnimationMode::Settled);
        assert_eq!(
            app.animator.current_frame(),
            AnimationSet::Sweep.frame_count() - 1
        );
    }

    #[test]
    fn error_settles_the_sweep_animation() {
        let mut app = App::new(AppCommand::Clean);
        app.on_event(ExecEvent::Error("test failure".into()));
        assert_eq!(app.execution, ExecutionState::Error);
        assert_eq!(app.animator.set(), AnimationSet::Sweep);
        assert_eq!(app.animator.mode(), AnimationMode::Settled);
    }

    #[test]
    fn animation_sweeps_only_while_running() {
        let idle = App::new(AppCommand::Home);
        assert_eq!(idle.animator.set(), AnimationSet::Idle);
        assert_eq!(idle.animator.mode(), AnimationMode::Ambient);
        let mut running = App::new(AppCommand::Clean);
        assert_eq!(running.animator.set(), AnimationSet::Sweep);
        assert_eq!(running.animator.mode(), AnimationMode::Sweeping);
        running.execution = ExecutionState::Done;
        running.sync_animation();
        assert_eq!(running.animator.mode(), AnimationMode::Settled);
    }

    #[test]
    fn starting_and_finishing_maintenance_switch_sets_safely() {
        let mut app = App::new(AppCommand::Home);
        for t in [1_000u64, 2_000, 3_000, 4_000] {
            app.animator.tick(t);
        }
        assert_eq!(app.animator.set(), AnimationSet::Idle);

        app.mark_started(1);
        assert_eq!(app.screen, Screen::Home);
        assert_eq!(app.animator.set(), AnimationSet::Sweep);
        assert_eq!(app.animator.mode(), AnimationMode::Sweeping);
        assert!(app.animator.current_frame() < AnimationSet::Sweep.frame_count());

        app.execution = ExecutionState::Done;
        app.sync_animation();
        assert_eq!(app.animator.set(), AnimationSet::Sweep);
        assert_eq!(app.animator.mode(), AnimationMode::Settled);
        assert_eq!(
            app.animator.current_frame(),
            AnimationSet::Sweep.frame_count() - 1
        );

        app.execution = ExecutionState::Idle;
        app.sync_animation();
        assert_eq!(app.animator.set(), AnimationSet::Idle);
        assert_eq!(app.animator.mode(), AnimationMode::Ambient);
        assert!(app.animator.current_frame() < AnimationSet::Idle.frame_count());
    }

    #[test]
    fn command_entry_editing_is_safe() {
        let mut app = App::new(AppCommand::Home);
        assert!(!app.input_focused);
        app.set_input_focus(true);
        assert!(app.input_focused);
        app.push_input("Akari");
        app.push_input("Clean");
        assert_eq!(app.input, "AkariClean");
        app.backspace_input();
        assert_eq!(app.input, "AkariClea");
        app.clear_input();
        assert!(app.input.is_empty());
        // Control characters are ignored, never executed.
        app.push_input("\n\t");
        assert!(app.input.is_empty());
        app.set_input_focus(false);
        assert!(!app.input_focused);
    }

    #[test]
    fn finishing_a_workflow_spawns_nothing() {
        use crate::system::RunTotals;
        let mut app = App::new(AppCommand::Clean);
        app.mark_started(1);
        assert!(app.worker_running);
        let summary = AppSummary::compute(
            "completed".into(),
            "not run - cleanup only".into(),
            false,
            RunTotals::default(),
            vec![],
            vec![],
            None,
            None,
        );
        assert!(app.on_event(ExecEvent::Finished(summary)));
        assert_eq!(app.execution, ExecutionState::Done);
        assert!(!app.worker_running);
        // No further events are produced by completion itself.
    }

    // --- Progress model regression tests (prompt §4.2) ---

    fn progress_app(total: u64) -> App {
        let mut app = App::new(AppCommand::Clean);
        app.mark_started(1);
        app.on_event(ExecEvent::Progress(ProgressState::new(
            0,
            total,
            "Starting…".to_string(),
        )));
        app
    }

    fn ok_result(id: &str) -> OpResult {
        OpResult::ok(id, id, id, "done".to_string())
    }

    #[test]
    fn progress_begins_at_initial_state() {
        let app = progress_app(39);
        assert_eq!(app.progress.completed, 0);
        assert_eq!(app.progress.total, 39);
        assert_eq!(app.progress.fraction(), 0.0);
        assert!(!app.progress.failed);
        assert!(!app.progress.cancelled);
    }

    #[test]
    fn completed_tasks_advance_progress() {
        let mut app = progress_app(4);
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            app.on_event(ExecEvent::OpFinished(ok_result(id)));
            assert_eq!(app.progress.completed, (i + 1) as u64);
        }
        assert_eq!(app.progress.fraction(), 0.75);
    }

    #[test]
    fn intermediate_advances_only_on_evidence() {
        // OpStarted (working) must not advance; only OpFinished does.
        let mut app = progress_app(4);
        app.on_event(ExecEvent::OpStarted {
            id: "x".into(),
            label: "X".into(),
            target: "t".into(),
        });
        assert_eq!(app.progress.completed, 0);
        assert!(app.progress.indeterminate);
        assert!(app.progress.current.contains('X'));
        // A bare Progress snapshot with the same completed must not invent
        // intermediate percentages either.
        app.on_event(ExecEvent::Progress(ProgressState::new(
            0,
            4,
            "X — working…".to_string(),
        )));
        assert_eq!(app.progress.completed, 0);
        app.on_event(ExecEvent::OpFinished(ok_result("x")));
        assert_eq!(app.progress.completed, 1);
    }

    #[test]
    fn opaque_tasks_hold_without_fabrication() {
        // While an opaque DISM runs, the bar holds at the last defensible
        // percentage and names the task; it never creeps upward on its own.
        let mut app = progress_app(6);
        app.on_event(ExecEvent::OpFinished(ok_result("r1")));
        app.on_event(ExecEvent::OpFinished(ok_result("r2")));
        assert!((app.progress.percent() - 100.0 * 2.0 / 6.0).abs() < 1e-9);
        app.on_event(ExecEvent::OpStarted {
            id: "repair:restorehealth".into(),
            label: "[Repair 4/6] DISM - REPAIR".into(),
            target: "DISM.exe /RestoreHealth".into(),
        });
        // Still 2/6 (33.3%), now marked working.
        assert_eq!(app.progress.completed, 2);
        assert!(app.progress.indeterminate);
        let line = app.progress.status_line(12);
        assert!(line.contains("33.3%"));
        assert!(line.contains("working:"));
        assert!(!line.contains("100.0%"));
    }

    #[test]
    fn combined_progress_spans_both_phases_without_resetting() {
        // Combined total 45: cleanup 39 units then health 6 units.
        let mut app = progress_app(45);
        for i in 0..39 {
            app.on_event(ExecEvent::OpFinished(ok_result(&format!("c{i}"))));
        }
        assert_eq!(app.progress.completed, 39);
        // Per-phase init with a smaller total must not reset the bar.
        app.on_event(ExecEvent::Progress(ProgressState::new(
            0,
            6,
            "Starting system repair…".to_string(),
        )));
        assert_eq!(app.progress.completed, 39);
        assert_eq!(app.progress.total, 45);
        // Health completions continue from 39 upward (never backward).
        let mut last = app.progress.completed;
        for i in 0..6 {
            app.on_event(ExecEvent::OpFinished(ok_result(&format!("r{i}"))));
            assert!(app.progress.completed >= last);
            last = app.progress.completed;
        }
        assert_eq!(app.progress.completed, 45);
    }

    #[test]
    fn progress_never_moves_backward() {
        let mut app = progress_app(10);
        app.on_event(ExecEvent::OpFinished(ok_result("a")));
        app.on_event(ExecEvent::OpFinished(ok_result("b")));
        assert_eq!(app.progress.completed, 2);
        // Stale/duplicate snapshot with fewer completions is ignored.
        app.on_event(ExecEvent::Progress(ProgressState::new(
            1,
            10,
            "stale".to_string(),
        )));
        assert_eq!(app.progress.completed, 2);
    }

    #[test]
    fn failure_does_not_appear_as_success() {
        let mut app = progress_app(3);
        app.on_event(ExecEvent::OpFinished(ok_result("a")));
        let mut bad = OpResult::failed("b", "B", "t", "boom".into());
        bad.exit_code = Some(1);
        app.on_event(ExecEvent::OpFinished(bad));
        assert!(app.progress.failed);
        app.on_event(ExecEvent::OpFinished(ok_result("c")));
        // All work accounted for (3/3) but flagged FAILED, not success.
        let totals = {
            let mut t = RunTotals::default();
            t.add_ok();
            t.add_error("boom".into());
            t
        };
        let summary = AppSummary::compute(
            "completed".into(),
            "ran".into(),
            true,
            totals,
            vec![],
            vec![],
            None,
            None,
        );
        app.on_event(ExecEvent::Finished(summary));
        assert_eq!(app.progress.completed, app.progress.total);
        assert!(app.progress.failed);
        let line = app.progress.status_line(10);
        assert!(line.contains("FAILED"));
    }

    #[test]
    fn cancellation_stays_below_full_and_marked() {
        let mut app = progress_app(6);
        app.on_event(ExecEvent::OpFinished(ok_result("a")));
        app.on_event(ExecEvent::OpFinished(ok_result("b")));
        let mut p = ProgressState::new(2, 6, "DISM ScanHealth".to_string());
        p.cancelled = true;
        app.on_event(ExecEvent::Progress(p));
        assert!(app.progress.cancelled);
        assert!(app.progress.completed < app.progress.total);
        let line = app.progress.status_line(10);
        assert!(line.contains("cancelled"));
        assert!(!line.contains("100.0%"));
    }

    #[test]
    fn reaches_full_only_at_terminal_completion() {
        let mut app = progress_app(2);
        app.on_event(ExecEvent::OpFinished(ok_result("a")));
        assert!(app.progress.completed < app.progress.total);
        assert_ne!(app.progress.percent_text(), "100.0%");
        let summary = AppSummary::compute(
            "completed".into(),
            "ran".into(),
            false,
            RunTotals::default(),
            vec![],
            vec![],
            None,
            None,
        );
        app.on_event(ExecEvent::Finished(summary));
        assert_eq!(app.progress.completed, app.progress.total);
        assert_eq!(app.progress.percent_text(), "100.0%");
    }

    #[test]
    fn fast_commands_without_output_do_not_break_progress() {
        let mut app = progress_app(3);
        // No stdout, immediate exit 0 — still one evidence unit each.
        for id in ["fast1", "fast2", "fast3"] {
            app.on_event(ExecEvent::OpStarted {
                id: id.into(),
                label: id.into(),
                target: "t".into(),
            });
            app.on_event(ExecEvent::OpFinished(ok_result(id)));
        }
        assert_eq!(app.progress.completed, 3);
        assert_eq!(app.progress.percent_text(), "100.0%");
        assert!(!app.progress.failed);
    }

    #[test]
    fn progress_updates_do_not_hide_final_result() {
        let mut app = progress_app(1);
        app.on_event(ExecEvent::OpFinished(ok_result("only")));
        let summary = AppSummary::compute(
            "completed".into(),
            "not run".into(),
            false,
            RunTotals::default(),
            vec![ok_result("only")],
            vec![],
            None,
            None,
        );
        assert!(app.on_event(ExecEvent::Finished(summary)));
        assert!(app.summary.is_some());
        assert_eq!(app.execution, ExecutionState::Done);
        assert_eq!(app.progress.completed, app.progress.total);
    }

    #[test]
    fn progress_event_burst_stays_responsive() {
        // 200 synthetic completions must drain quickly (generous bound —
        // not a brittle benchmark, just a stall guard). Real runs emit
        // ~45 completions; per-file chatter would be orders more.
        use std::time::Instant;
        let mut app = progress_app(200);
        let t0 = Instant::now();
        for i in 0..200 {
            app.on_event(ExecEvent::OpFinished(ok_result(&format!("u{i}"))));
        }
        let ms = t0.elapsed().as_millis();
        assert_eq!(app.progress.completed, 200);
        assert!(ms < 2000, "200 progress updates took {ms} ms");
    }
}
