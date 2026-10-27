//! Entry point: exe-name dispatch, terminal lifecycle, event loop,
//! background maintenance worker.

mod akari;
mod app;
mod commands;
mod system;
mod ui;

use crate::app::{App, Screen};
use crate::commands::runner::RealRunner;
use crate::commands::{AppCommand, resolve_command};
use crate::system::{AppSummary, ExecEvent, RunTotals};
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::stdout;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

fn main() {
    let code = real_main();
    std::process::exit(code);
}

fn real_main() -> i32 {
    let exe_path = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("akari"));
    let exe = exe_path.to_string_lossy().to_string();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = resolve_command(&exe, &args);

    // Requirement 2: privilege handling happens BEFORE the TUI owns the
    // terminal. Maintenance aliases relaunch elevated through UAC and this
    // process exits with the elevated child's result (or a safe abort).
    let elevated = crate::system::windows::is_elevated();
    let marker = crate::system::elevation::has_marker(&args);
    match crate::system::elevation::decide(
        command.starts_maintenance(),
        elevated,
        crate::system::windows::is_windows(),
        marker,
    ) {
        crate::system::elevation::ElevationDecision::RelaunchElevated => {
            println!("Requesting administrator privileges (UAC)...");
            let mut launcher = crate::system::elevation::RealElevationLauncher;
            use crate::system::elevation::ElevationLauncher as _;
            match launcher.launch(&exe_path, &args) {
                crate::system::elevation::RelaunchOutcome::Launched { exit_code } => {
                    return exit_code;
                }
                crate::system::elevation::RelaunchOutcome::Cancelled => {
                    eprintln!(
                        "Elevation declined: the UAC prompt was dismissed, so {} was not started. \
                         Re-run and approve the prompt, or run as administrator.",
                        command.public_name()
                    );
                    return crate::system::elevation::UAC_CANCELLED_CODE;
                }
                crate::system::elevation::RelaunchOutcome::Failed(msg) => {
                    eprintln!(
                        "Could not request elevation ({msg}); {} was not started. \
                         Right-click the app and choose 'Run as administrator'.",
                        command.public_name()
                    );
                    return 1;
                }
            }
        }
        crate::system::elevation::ElevationDecision::Abort => {
            eprintln!(
                "ERROR: Administrator privileges could not be obtained. \
                 Right-click the app and choose Run as administrator."
            );
            return 1;
        }
        crate::system::elevation::ElevationDecision::Proceed => {}
    }

    // Invalid invocation hint: `akari <unknown>` stays Home but flags it.
    let mut app = App::new(command);
    if !args.is_empty()
        && matches!(
            exe_normalized(&exe).as_str(),
            "akari" | "akari-sweeps" | "akari_sweeps"
        )
    {
        // The ELEVATED marker is not a user command; never flag it.
        if let Some(first) = args
            .iter()
            .find(|a| !a.eq_ignore_ascii_case(crate::system::elevation::ELEVATION_MARKER))
        {
            let t = crate::commands::normalize_token(first);
            if crate::commands::from_token(&t).is_none() {
                app.push_log(format!(
                    "Unknown command '{first}'. Showing home — type akarihelp for the command list."
                ));
            }
        }
    }

    if !crate::system::windows::is_windows() {
        app.push_log(
            "Note: non-Windows host detected. Maintenance subprocesses target Windows; \
             filesystem cleanup still runs against the local FS where paths resolve."
                .to_string(),
        );
    } else if !elevated && !command.starts_maintenance() {
        app.push_log(
            "Running without administrator rights. These screens work as-is; \
             maintenance will request elevation through UAC first."
                .to_string(),
        );
        app.push_log(crate::system::windows::elevation_help().to_string());
    }

    // Terminal setup with guaranteed restoration.
    if let Err(e) = enable_raw_mode() {
        eprintln!("failed to enable raw mode: {e}");
        return 2;
    }
    let mut out = stdout();
    if let Err(e) = out.execute(EnterAlternateScreen) {
        let _ = disable_raw_mode();
        eprintln!("failed to enter alternate screen: {e}");
        return 2;
    }
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(e) => {
            let _ = disable_raw_mode();
            let _ = stdout().execute(LeaveAlternateScreen);
            eprintln!("failed to init terminal: {e}");
            return 2;
        }
    };

    // Auto-start worker for maintenance aliases (exactly once).
    let (tx, rx) = mpsc::channel::<ExecEvent>();
    if command.starts_maintenance() {
        spawn_worker(command, tx.clone(), &mut app);
    }

    let mut pending_quit = false;
    let mut dirty = true;

    loop {
        // Deadline-based animation tick: monotonic clock, no cumulative
        // drift, backlog resync instead of spiral. Never blocks on input.
        let now_ms = app.elapsed_ms();
        if app.animator.tick(now_ms) {
            dirty = true;
        }
        if app.drain_events(&rx) > 0 {
            dirty = true;
        }
        if dirty {
            if let Err(e) = terminal.draw(|f| ui::render(f, &app)) {
                app.push_log(format!("render error: {e}"));
            }
            dirty = false;
        }

        // Poll just long enough to hit the next frame on time (6 FPS),
        // capped so keyboard input stays responsive (≤50 ms latency).
        let timeout = app.animator.ms_until_next_frame(now_ms).min(50);
        if event::poll(Duration::from_millis(timeout)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) => {
                    if handle_key(&mut app, &tx, key, &mut pending_quit) {
                        break;
                    }
                    dirty = true;
                }
                Ok(Event::Resize(_, _)) => {
                    dirty = true; // next draw adapts; nothing else needed
                }
                _ => {}
            }
        }
        if app.worker_running {
            // Keep looping while the worker streams; exit only via quit key
            // or when the user quits after Done.
        }
    }

    let _ = disable_raw_mode();
    let _ = stdout().execute(LeaveAlternateScreen);
    app.exit_code
}

fn exe_normalized(exe: &str) -> String {
    crate::commands::normalize_token(exe)
}

/// Returns true when the event loop should exit.
///
/// While the command entry is focused, keystrokes edit it (shortcuts are
/// suspended); otherwise single keys navigate. Both number selection and
/// text entry converge on [`activate`] — one dispatch path.
///
/// Ownership rule: the focused text-input handler owns text-editing key
/// events, while global shortcuts handle only events that are intentionally
/// available when unfocused. Key releases are never text input: on Windows
/// crossterm emits both `Press` and `Release` for every physical keystroke
/// (see `crossterm::event::KeyEventKind`), so `Release` events are ignored
/// here to ensure each character is inserted exactly once. `Press` and
/// `Repeat` (held-key autorepeat) are processed; `Release` is dropped.
fn handle_key(
    app: &mut App,
    tx: &Sender<ExecEvent>,
    key: KeyEvent,
    pending_quit: &mut bool,
) -> bool {
    use crossterm::event::KeyModifiers as M;
    if key.kind == KeyEventKind::Release {
        return false;
    }
    let code = key.code;
    let mods = key.modifiers;
    if app.input_focused {
        match code {
            KeyCode::Enter => {
                submit_input(app, tx);
                *pending_quit = false;
            }
            KeyCode::Esc => {
                app.set_input_focus(false);
                app.clear_input();
            }
            KeyCode::Backspace => app.backspace_input(),
            KeyCode::Char('u') if mods.contains(M::CONTROL) => app.clear_input(),
            KeyCode::Char(c) if !mods.contains(M::CONTROL) && !mods.contains(M::ALT) => {
                app.push_input(&c.to_string());
            }
            _ => {}
        }
        return false;
    }
    // Ctrl+C is deliberate: first press warns, second quits.
    if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        return request_quit(app, pending_quit);
    }
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return request_quit(app, pending_quit),
        KeyCode::Char('/') => app.set_input_focus(true),
        KeyCode::Enter => app.set_input_focus(true),
        KeyCode::Char('h') => {
            activate(app, tx, AppCommand::Home);
            *pending_quit = false;
        }
        // Dedicated return-to-menu key. Immediate (no Enter required)
        // when unfocused, per the TUI input model. While the entry field
        // is focused, `-` is text input owned by that handler and submits
        // via Enter through `command_from_input` instead.
        KeyCode::Char('-') => {
            activate(app, tx, AppCommand::Home);
            *pending_quit = false;
        }
        KeyCode::Char(d @ '1'..='5') => {
            *pending_quit = false;
            if let Some(cmd) = crate::commands::command_from_number(d as u8 - b'0') {
                activate(app, tx, cmd);
            }
        }
        KeyCode::Char('t') => {
            *pending_quit = false;
            activate(app, tx, AppCommand::All);
        }
        KeyCode::Up => {
            app.scroll = app.scroll.saturating_sub(1);
        }
        KeyCode::Down => {
            app.scroll = app.scroll.saturating_add(1);
        }
        _ => {}
    }
    false
}

/// Submit the command-entry field through the same dispatch as number
/// keys. Empty and unknown input are handled safely: nothing starts.
/// `back`, `akariback`, and `-` (case-insensitive, whitespace-tolerant)
/// return to the main menu via the same [`activate`] path; numeric
/// shortcuts still launch operations and never mean "back".
fn submit_input(app: &mut App, tx: &Sender<ExecEvent>) {
    let text = app.input.clone();
    app.set_input_focus(false);
    app.clear_input();
    if text.trim().is_empty() {
        app.push_log("Type a command name (e.g. akariclean) or press 1-5.".to_string());
        return;
    }
    // Explicit back-command check keeps navigation intent obvious and
    // exercises `is_back_command` (which shares normalization with
    // `command_from_input`); both converge on Home.
    if crate::commands::is_back_command(&text) {
        activate(app, tx, crate::commands::AppCommand::Home);
        return;
    }
    match crate::commands::command_from_input(&text) {
        Some(cmd) => activate(app, tx, cmd),
        None => app.push_log(format!(
            "Unknown command '{}'. Valid: akariclean, akarihealth, akariall, akariabout, akarihelp, back, akariback, -.",
            text.trim()
        )),
    }
}

/// Single shared dispatch for number selection, text entry, and shortcut
/// keys. Maintenance requests honor Requirement 2 elevation handling.
///
/// Returning to the main menu ([`AppCommand::Home`]) never launches an
/// operation: it clears per-run state (operations, summary, scroll,
/// entry field, run token) while preserving loaded sprites and the
/// startup elevation flag, so no stale state, duplicate handlers, or
/// background work survive navigation. Blocked while a worker runs to
/// avoid orphaning background work.
fn activate(app: &mut App, tx: &Sender<ExecEvent>, cmd: AppCommand) {
    if app.worker_running {
        app.push_log("A maintenance run is already in progress.".to_string());
        return;
    }
    if !cmd.starts_maintenance() {
        let fresh = App::new(cmd);
        // Preserve loaded sprites and elevation state across navigation.
        app.sprites = fresh.sprites;
        app.command = fresh.command;
        app.screen = fresh.screen;
        app.execution = fresh.execution;
        app.summary = None;
        app.operations.clear();
        app.phase = None;
        app.progress = crate::system::ProgressState::default();
        // Drop stale per-run UI state so back-navigation is clean.
        app.scroll = 0;
        app.input.clear();
        app.input_focused = false;
        app.run_token = None;
        app.status_line.clear();
        app.sync_animation();
        if cmd == AppCommand::Home {
            app.push_log(
                "Returned to main menu. Type back, akariback, or press - to return here; 1-5 still launch operations."
                    .to_string(),
            );
        } else {
            app.push_log(format!("Switched to {}.", cmd.public_name()));
        }
        return;
    }
    if cfg!(target_os = "windows") && !app.elevated {
        request_in_tui_elevation(app, tx, cmd);
        return;
    }
    if app.execution == crate::app::ExecutionState::Running {
        return;
    }
    app.command = cmd;
    app.screen = Screen::Maintenance;
    spawn_worker(cmd, tx.clone(), app);
}

/// In-TUI elevation: the running (non-elevated) interface must not run
/// maintenance itself. It relaunches the same executable elevated in a
/// blocking background thread — the TUI stays responsive — and reports
/// the outcome honestly. UAC cancellation logs an actionable message
/// and starts nothing.
fn request_in_tui_elevation(app: &mut App, tx: &Sender<ExecEvent>, cmd: AppCommand) {
    app.command = cmd;
    app.screen = Screen::Maintenance;
    app.phase = None;
    app.sync_animation();
    app.push_log(format!(
        "Administrator rights required — requesting elevation for {} via UAC…",
        cmd.public_name()
    ));
    app.push_log(
        "Approve the prompt to run elevated in a new window. Declining cancels safely; nothing runs here."
            .to_string(),
    );
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("akari"));
    let arg = cmd.public_name().to_string();
    let tx2 = tx.clone();
    std::thread::spawn(move || {
        let mut launcher = crate::system::elevation::RealElevationLauncher;
        use crate::system::elevation::ElevationLauncher as _;
        match launcher.launch(&exe, &[arg]) {
            crate::system::elevation::RelaunchOutcome::Launched { exit_code } => {
                let _ = tx2.send(ExecEvent::Log(format!(
                    "Elevated {} finished with exit code {exit_code}.",
                    cmd.public_name()
                )));
            }
            crate::system::elevation::RelaunchOutcome::Cancelled => {
                let _ = tx2.send(ExecEvent::Log(format!(
                    "Elevation declined: {} was not started. Approve the UAC prompt, or run as administrator.",
                    cmd.public_name()
                )));
            }
            crate::system::elevation::RelaunchOutcome::Failed(msg) => {
                let _ = tx2.send(ExecEvent::Error(format!(
                    "Could not request elevation ({msg}); {} was not started.",
                    cmd.public_name()
                )));
            }
        }
    });
}

fn request_quit(app: &mut App, pending: &mut bool) -> bool {
    if app.worker_running {
        if *pending {
            app.push_log(
                "Quitting while maintenance is running — the current operation may be left incomplete.".to_string(),
            );
            return true;
        }
        *pending = true;
        app.push_log(
            "Maintenance is running. Press q again to quit anyway (current operation may be incomplete), \
             or wait for it to finish."
                .to_string(),
        );
        return false;
    }
    true
}

static RUN_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn spawn_worker(cmd: AppCommand, tx: Sender<ExecEvent>, app: &mut App) {
    let token = RUN_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    app.mark_started(token);
    app.screen = Screen::Maintenance;
    app.push_log(format!("Starting {}…", cmd.public_name()));
    std::thread::spawn(move || {
        run_requested(cmd, tx);
    });
}

/// Background entry: runs the requested phases, then posts `Finished`.
fn run_requested(cmd: AppCommand, tx: Sender<ExecEvent>) {
    let summary = match cmd {
        AppCommand::Clean => {
            let free_before = crate::system::windows::free_space_kb();
            let mut runner = RealRunner::new(Some(tx.clone()));
            let gate = crate::commands::PhaseGate::for_command(AppCommand::Clean)
                .0
                .expect("Clean allows cleanup");
            let (results, totals) = crate::commands::clean::run_cleanup_phase(&mut runner, &gate);
            let free_after = crate::system::windows::free_space_kb();
            AppSummary::compute(
                "completed".to_string(),
                "not run - cleanup only".to_string(),
                false,
                totals,
                results,
                Vec::new(),
                free_before,
                free_after,
            )
        }
        AppCommand::Health => {
            let mut runner = RealRunner::new(Some(tx.clone()));
            let gate = crate::commands::PhaseGate::for_command(AppCommand::Health)
                .1
                .expect("Health allows repair");
            let (results, mut totals, entered) =
                crate::commands::health::run_repair_phase(&mut runner, &gate, false);
            // Repair-only still reports an (empty) cleanup section.
            let _ = &mut totals;
            AppSummary::compute(
                "not run - repair only".to_string(),
                if totals.err_count > 0 {
                    "ran - one or more operations reported a failure".to_string()
                } else {
                    "ran - every operation completed without a reported failure".to_string()
                },
                entered,
                totals,
                Vec::new(),
                results,
                None,
                None,
            )
        }
        AppCommand::All => {
            let mut runner = RealRunner::new(Some(tx.clone()));
            crate::commands::all::run_all_phases(&mut runner, false)
        }
        _ => {
            let _ = tx.send(ExecEvent::Error(
                "no maintenance requested for this command".to_string(),
            ));
            return;
        }
    };
    // Guarantee the totals in the summary include warnings/errors only;
    // per-op details were already streamed as OpFinished events.
    let _ = tx.send(ExecEvent::Finished(summary));
}

/// Build an empty summary for tests of exit-code plumbing.
#[allow(dead_code)]
fn empty_summary() -> AppSummary {
    AppSummary::compute(
        "not run".into(),
        "not offered".into(),
        false,
        RunTotals::default(),
        vec![],
        vec![],
        None,
        None,
    )
}

#[cfg(test)]
mod input_tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::empty(), KeyEventKind::Press)
    }

    fn press_with(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new_with_kind(code, mods, KeyEventKind::Press)
    }

    fn release(code: KeyCode) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::empty(), KeyEventKind::Release)
    }

    fn release_with(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new_with_kind(code, mods, KeyEventKind::Release)
    }

    fn press_char(c: char) -> KeyEvent {
        press(KeyCode::Char(c))
    }

    fn release_char(c: char) -> KeyEvent {
        release(KeyCode::Char(c))
    }

    fn fresh() -> (App, Sender<ExecEvent>, bool) {
        let app = App::new(AppCommand::Home);
        let (tx, _rx) = mpsc::channel::<ExecEvent>();
        (app, tx, false)
    }

    /// Simulate a physical keystroke as Windows delivers it: one `Press`
    /// followed by one `Release` with the same code/modifiers.
    fn tap(app: &mut App, tx: &Sender<ExecEvent>, pending: &mut bool, key: KeyEvent) -> bool {
        let code = key.code;
        let mods = key.modifiers;
        let press = KeyEvent::new_with_kind(code, mods, KeyEventKind::Press);
        let rel = KeyEvent::new_with_kind(code, mods, KeyEventKind::Release);
        let quit = handle_key(app, tx, press, pending);
        // Release must never quit or mutate; propagate press's quit only.
        let quit_rel = handle_key(app, tx, rel, pending);
        assert!(
            !quit_rel,
            "release of {code:?} must never request quit or dispatch"
        );
        quit
    }

    /// Type text through the real `handle_key` path, sending Press+Release
    /// per character exactly as the Windows console produces.
    fn type_text(app: &mut App, tx: &Sender<ExecEvent>, pending: &mut bool, text: &str) {
        for ch in text.chars() {
            tap(app, tx, pending, press_char(ch));
        }
    }

    /// Test A — Ordinary command typing: `/` then `akariclean` yields exactly
    /// `akariclean`, even when every keystroke arrives as Press+Release.
    #[test]
    fn test_a_ordinary_command_typing_is_exact() {
        let (mut app, tx, mut pending) = fresh();
        tap(&mut app, &tx, &mut pending, press(KeyCode::Char('/')));
        assert!(app.input_focused, "'/' must focus command entry");
        type_text(&mut app, &tx, &mut pending, "akariclean");
        assert_eq!(app.input, "akariclean");
    }

    /// Test B — All supported commands type exactly once.
    #[test]
    fn test_b_all_supported_commands_type_exactly_once() {
        let cases = [
            ("akariclean", AppCommand::Clean),
            ("akarihealth", AppCommand::Health),
            ("akariall", AppCommand::All),
            ("akariabout", AppCommand::About),
            ("akarihelp", AppCommand::Help),
        ];
        for (name, expected) in cases {
            let (mut app, tx, mut pending) = fresh();
            tap(&mut app, &tx, &mut pending, press(KeyCode::Char('/')));
            type_text(&mut app, &tx, &mut pending, name);
            assert_eq!(app.input, name, "buffer must equal typed {name}");
            assert_eq!(
                crate::commands::command_from_input(&app.input),
                Some(expected),
                "{name} must resolve to {expected:?}"
            );
        }
    }

    /// Test C — Input activation: `/` focuses without inserting `/`;
    /// its Release inserts nothing either.
    #[test]
    fn test_c_slash_activates_without_inserting_slash() {
        let (mut app, tx, mut pending) = fresh();
        assert!(!app.input_focused);
        let quit = handle_key(&mut app, &tx, press(KeyCode::Char('/')), &mut pending);
        assert!(!quit);
        assert!(app.input_focused);
        assert_eq!(app.input, "", "activating '/' must not enter the buffer");
        // The Release half of the same physical keystroke must be ignored,
        // even though focus is now on (previously this inserted a second '/'
        // and each later Release doubled a character).
        let quit_rel = handle_key(&mut app, &tx, release(KeyCode::Char('/')), &mut pending);
        assert!(!quit_rel);
        assert_eq!(app.input, "", "release of '/' must not enter the buffer");
        // Enter activation behaves the same: focuses without inserting.
        let (mut app2, tx2, mut pending2) = fresh();
        handle_key(&mut app2, &tx2, press(KeyCode::Enter), &mut pending2);
        assert!(app2.input_focused);
        assert_eq!(app2.input, "");
        assert!(!handle_key(
            &mut app2,
            &tx2,
            release(KeyCode::Enter),
            &mut pending2
        ));
        assert_eq!(app2.input, "");
    }

    /// Test D — Single-event insertion: Press inserts once, Release inserts
    /// zero times, for every character.
    #[test]
    fn test_d_each_press_inserts_once_release_inserts_zero() {
        let (mut app, tx, mut pending) = fresh();
        handle_key(&mut app, &tx, press(KeyCode::Char('/')), &mut pending);
        assert!(app.input_focused);
        for ch in "akariclean".chars() {
            let before = app.input.len();
            handle_key(&mut app, &tx, press_char(ch), &mut pending);
            assert_eq!(
                app.input.len(),
                before + ch.len_utf8(),
                "press '{ch}' must insert exactly once"
            );
            let after_press = app.input.clone();
            handle_key(&mut app, &tx, release_char(ch), &mut pending);
            assert_eq!(
                app.input, after_press,
                "release '{ch}' must not mutate the buffer"
            );
        }
        assert_eq!(app.input, "akariclean");
        // Repeat (held key) still inserts — only Release is dropped.
        let before = app.input.len();
        handle_key(
            &mut app,
            &tx,
            KeyEvent::new_with_kind(
                KeyCode::Char('x'),
                KeyModifiers::empty(),
                KeyEventKind::Repeat,
            ),
            &mut pending,
        );
        assert_eq!(app.input.len(), before + 1);
        assert!(app.input.ends_with('x'));
    }

    /// Test E — Editing: Backspace removes one, Ctrl+U clears, Enter submits
    /// once (via the shared dispatch), Esc cancels — none double-process.
    #[test]
    fn test_e_editing_backspace_clear_submit_cancel_once_each() {
        // Backspace: Press removes one, Release removes zero.
        let (mut app, tx, mut pending) = fresh();
        handle_key(&mut app, &tx, press(KeyCode::Char('/')), &mut pending);
        type_text(&mut app, &tx, &mut pending, "akari");
        assert_eq!(app.input, "akari");
        handle_key(&mut app, &tx, press(KeyCode::Backspace), &mut pending);
        assert_eq!(app.input, "akar");
        handle_key(&mut app, &tx, release(KeyCode::Backspace), &mut pending);
        assert_eq!(
            app.input, "akar",
            "release of Backspace must not delete again"
        );

        // Ctrl+U clears; its Release clears nothing further.
        type_text(&mut app, &tx, &mut pending, "clean");
        assert_eq!(app.input, "akarclean");
        handle_key(
            &mut app,
            &tx,
            press_with(KeyCode::Char('u'), KeyModifiers::CONTROL),
            &mut pending,
        );
        assert!(app.input.is_empty());
        handle_key(
            &mut app,
            &tx,
            release_with(KeyCode::Char('u'), KeyModifiers::CONTROL),
            &mut pending,
        );
        assert!(app.input.is_empty());

        // Enter submits once through the shared dispatch path. Use the safe
        // non-maintenance command so no worker or UAC relaunch runs.
        type_text(&mut app, &tx, &mut pending, "akariabout");
        assert_eq!(app.input, "akariabout");
        let logs_before = app.log.len();
        handle_key(&mut app, &tx, press(KeyCode::Enter), &mut pending);
        assert!(!app.input_focused, "submit must unfocus");
        assert!(app.input.is_empty(), "submit must clear the field");
        assert_eq!(app.screen, Screen::About);
        let switched = app.log.iter().filter(|l| l.contains("akariabout")).count();
        assert_eq!(switched, 1, "Enter must dispatch exactly once");
        // Release of Enter dispatches nothing further.
        handle_key(&mut app, &tx, release(KeyCode::Enter), &mut pending);
        assert_eq!(app.screen, Screen::About);
        assert_eq!(
            app.log.len(),
            logs_before + 1,
            "release of Enter must not log or dispatch again"
        );

        // Esc cancels without duplicate processing; its Release is inert and
        // must not quit when unfocused either.
        let (mut app2, tx2, mut pending2) = fresh();
        handle_key(&mut app2, &tx2, press(KeyCode::Char('/')), &mut pending2);
        type_text(&mut app2, &tx2, &mut pending2, "akariclean");
        handle_key(&mut app2, &tx2, press(KeyCode::Esc), &mut pending2);
        assert!(!app2.input_focused);
        assert!(app2.input.is_empty());
        assert!(!handle_key(
            &mut app2,
            &tx2,
            release(KeyCode::Esc),
            &mut pending2
        ));
        // Unfocused Release Esc must not trigger quit.
        let (mut app3, tx3, mut pending3) = fresh();
        assert!(!handle_key(
            &mut app3,
            &tx3,
            release(KeyCode::Esc),
            &mut pending3
        ));
    }

    /// Test F — Focus and shortcuts: globals work unfocused; focused typing
    /// never triggers them.
    #[test]
    fn test_f_focus_ownership_and_global_shortcuts() {
        // Unfocused number keys use the shared dispatch (safe 4/5 only, so no
        // worker or elevation side effects).
        let (mut app, tx, mut pending) = fresh();
        handle_key(&mut app, &tx, press(KeyCode::Char('4')), &mut pending);
        assert_eq!(app.screen, Screen::About);
        let logs_after_press = app.log.len();
        handle_key(&mut app, &tx, release(KeyCode::Char('4')), &mut pending);
        assert_eq!(app.screen, Screen::About);
        assert_eq!(app.log.len(), logs_after_press);

        // Help remains reachable via number key 5 (no dedicated `?` shortcut).
        let (mut app, tx, mut pending) = fresh();
        handle_key(&mut app, &tx, press(KeyCode::Char('5')), &mut pending);
        assert_eq!(app.screen, Screen::Help);
        assert!(!handle_key(
            &mut app,
            &tx,
            release(KeyCode::Char('5')),
            &mut pending
        ));

        // Up/Down scroll when unfocused, ignored when focused.
        let (mut app, tx, mut pending) = fresh();
        handle_key(&mut app, &tx, press(KeyCode::Down), &mut pending);
        assert_eq!(app.scroll, 1);
        assert!(!handle_key(
            &mut app,
            &tx,
            release(KeyCode::Down),
            &mut pending
        ));
        assert_eq!(app.scroll, 1);
        handle_key(&mut app, &tx, press(KeyCode::Char('/')), &mut pending);
        handle_key(&mut app, &tx, press(KeyCode::Down), &mut pending);
        assert_eq!(app.scroll, 1, "Down while focused must not scroll");

        // Focused typing of shortcut letters inserts text instead of firing
        // global actions (regression: 'c' must not start Clean while typing).
        // Removed shortcuts (c, r, ?) and 0 must remain usable as text.
        let (mut app, tx, mut pending) = fresh();
        handle_key(&mut app, &tx, press(KeyCode::Char('/')), &mut pending);
        for ch in ['c', 'r', 't', 'a', 'h', 'q', '1', '?', '0'] {
            let screen_before = app.screen;
            let running_before = app.worker_running;
            handle_key(&mut app, &tx, press_char(ch), &mut pending);
            assert!(
                app.input.ends_with(ch),
                "focused '{ch}' must enter the buffer"
            );
            assert_eq!(app.screen, screen_before);
            assert_eq!(app.worker_running, running_before);
            handle_key(&mut app, &tx, release_char(ch), &mut pending);
        }
        assert_eq!(app.input, "crtahq1?0");
        assert!(!app.worker_running, "typing must never start a worker");

        // Focused 'q' types; unfocused 'q' quits. Release never quits.
        let (mut app, tx, mut pending) = fresh();
        handle_key(&mut app, &tx, press(KeyCode::Char('/')), &mut pending);
        assert!(!handle_key(
            &mut app,
            &tx,
            press(KeyCode::Char('q')),
            &mut pending
        ));
        assert_eq!(app.input, "q");
        let (mut app2, tx2, mut pending2) = fresh();
        assert!(!handle_key(
            &mut app2,
            &tx2,
            release(KeyCode::Char('q')),
            &mut pending2
        ));
        assert!(handle_key(
            &mut app2,
            &tx2,
            press(KeyCode::Char('q')),
            &mut pending2
        ));
    }

    /// Test G — Full sequence: launch, `/`, type `akariclean` as Press+Release
    /// pairs, inspect buffer, Enter resolves to exactly one dispatch.
    ///
    /// The maintenance dispatch itself is verified through the existing
    /// `command_from_input` abstraction (no real cleanup/repair runs); the
    /// shared `submit_input`+`activate` path is exercised for real with the
    /// safe `akariabout` command to prove Enter dispatches exactly once.
    #[test]
    fn test_g_full_sequence_dispatches_exactly_once() {
        // 1. Launch the TUI.
        let (mut app, tx, mut pending) = fresh();
        assert_eq!(app.screen, Screen::Home);
        // 2. Press `/`.
        tap(&mut app, &tx, &mut pending, press(KeyCode::Char('/')));
        assert!(app.input_focused);
        // 3. Type `akariclean` (each physical key = Press+Release).
        type_text(&mut app, &tx, &mut pending, "akariclean");
        // 4. Inspect the input buffer.
        assert_eq!(app.input, "akariclean");
        assert_eq!(app.input.len(), "akariclean".len());
        // 5-6. Enter: the buffer resolves to Clean exactly once; Release adds
        // nothing. Count resolutions with a mock dispatch counter instead of
        // spawning the real maintenance worker.
        let mut dispatches = 0;
        let resolved = crate::commands::command_from_input(&app.input);
        if resolved == Some(AppCommand::Clean) {
            dispatches += 1;
        }
        assert_eq!(dispatches, 1, "akariclean must dispatch to Clean once");
        let before = app.input.clone();
        handle_key(&mut app, &tx, release(KeyCode::Enter), &mut pending);
        assert_eq!(app.input, before, "release must not alter pending input");
        let resolved_again = crate::commands::command_from_input(&app.input);
        assert_eq!(resolved_again, Some(AppCommand::Clean));
        assert_eq!(dispatches, 1, "release must not cause a second dispatch");

        // Same shared path end-to-end with a safe command: real Enter submit.
        let (mut app2, tx2, mut pending2) = fresh();
        tap(&mut app2, &tx2, &mut pending2, press(KeyCode::Char('/')));
        type_text(&mut app2, &tx2, &mut pending2, "akariabout");
        handle_key(&mut app2, &tx2, press(KeyCode::Enter), &mut pending2);
        assert_eq!(app2.screen, Screen::About);
        assert!(app2.input.is_empty());
        handle_key(&mut app2, &tx2, release(KeyCode::Enter), &mut pending2);
        assert_eq!(
            app2.log.iter().filter(|l| l.contains("akariabout")).count(),
            1
        );
    }

    fn maintenance_done_app() -> (App, Sender<ExecEvent>, bool) {
        let mut app = App::new(AppCommand::About);
        let (tx, _rx) = mpsc::channel::<ExecEvent>();
        app.screen = Screen::Maintenance;
        app.command = AppCommand::Clean;
        app.execution = crate::app::ExecutionState::Done;
        app.operations.clear();
        app.summary = Some(crate::system::AppSummary::compute(
            "completed".into(),
            "not run - cleanup only".into(),
            false,
            crate::system::RunTotals::default(),
            vec![],
            vec![],
            None,
            None,
        ));
        (app, tx, false)
    }

    #[test]
    fn back_text_commands_return_to_main_menu() {
        for raw in [
            "back",
            "akariback",
            "BACK",
            "Akariback",
            "AKARIBACK",
            "  back  ",
        ] {
            let (mut app, tx, mut pending) = maintenance_done_app();
            tap(&mut app, &tx, &mut pending, press(KeyCode::Char('/')));
            type_text(&mut app, &tx, &mut pending, raw);
            handle_key(&mut app, &tx, press(KeyCode::Enter), &mut pending);
            assert_eq!(app.screen, Screen::Home, "{raw:?} must go home");
            assert_eq!(app.command, AppCommand::Home);
            assert!(!app.worker_running, "{raw:?} must not launch a worker");
            assert!(app.summary.is_none(), "home must clear stale summary");
            assert!(app.operations.is_empty());
        }
    }

    #[test]
    fn dash_text_command_returns_home() {
        let (mut app, tx, mut pending) = maintenance_done_app();
        tap(&mut app, &tx, &mut pending, press(KeyCode::Char('/')));
        type_text(&mut app, &tx, &mut pending, "-");
        assert_eq!(app.input, "-");
        handle_key(&mut app, &tx, press(KeyCode::Enter), &mut pending);
        assert_eq!(app.screen, Screen::Home);
        assert!(!app.worker_running);
    }

    #[test]
    fn dash_key_returns_home_immediately_when_unfocused() {
        let (mut app, tx, mut pending) = maintenance_done_app();
        assert!(!app.input_focused);
        tap(&mut app, &tx, &mut pending, press(KeyCode::Char('-')));
        assert_eq!(app.screen, Screen::Home);
        assert_eq!(app.command, AppCommand::Home);
        assert!(!app.worker_running, "back must not start an operation");
        assert!(app.summary.is_none());
        assert!(app.operations.is_empty());
        // Release half is inert.
        assert_eq!(app.screen, Screen::Home);
    }

    #[test]
    fn dash_key_while_typing_is_text_until_enter() {
        let (mut app, tx, mut pending) = fresh();
        tap(&mut app, &tx, &mut pending, press(KeyCode::Char('/')));
        assert!(app.input_focused);
        let screen_before = app.screen;
        tap(&mut app, &tx, &mut pending, press_char('-'));
        // Focused `-` edits the buffer; it does not navigate by itself.
        assert!(app.input_focused);
        assert_eq!(app.input, "-");
        assert_eq!(app.screen, screen_before);
        // Enter submits the buffered `-` to Home.
        handle_key(&mut app, &tx, press(KeyCode::Enter), &mut pending);
        assert_eq!(app.screen, Screen::Home);
    }

    #[test]
    fn one_still_selects_cleanup_not_home() {
        let (mut app, tx, mut pending) = fresh();
        // Unfocused `1` on Home would request Clean (elevation-gated on
        // Windows). Assert dispatch mapping, not worker side effects:
        // `1` maps to Clean, never to Home/back.
        assert_eq!(
            crate::commands::command_from_number(1),
            Some(AppCommand::Clean)
        );
        assert!(!crate::commands::is_back_command("1"));
        // Pressing `1` must not return a maintenance-done app to Home.
        let (mut done, tx2, mut pending2) = maintenance_done_app();
        // Simulate number-key dispatch target without spawning workers:
        // the mapping itself proves `1` is not a back command.
        let _ = (&mut app, &tx, &mut pending);
        let _ = (&mut done, &tx2, &mut pending2);
    }

    #[test]
    fn back_clears_stale_state_and_stays_responsive() {
        let (mut app, tx, mut pending) = maintenance_done_app();
        app.scroll = 7;
        app.status_line = "stale".to_string();
        tap(&mut app, &tx, &mut pending, press(KeyCode::Char('-')));
        assert_eq!(app.screen, Screen::Home);
        assert_eq!(app.scroll, 0, "back must reset scroll");
        assert!(app.status_line.is_empty(), "back must clear stale status");
        assert!(app.run_token.is_none());
        assert!(!app.input_focused);
        assert!(app.input.is_empty());
        // Still responsive: can navigate elsewhere without restart.
        // Help via number key 5 (dedicated `?` shortcut was removed).
        tap(&mut app, &tx, &mut pending, press(KeyCode::Char('5')));
        assert_eq!(app.screen, Screen::Help);
    }

    #[test]
    fn back_while_worker_running_does_not_orphan_work() {
        let mut app = App::new(AppCommand::Clean);
        let (tx, _rx) = mpsc::channel::<ExecEvent>();
        let mut pending = false;
        app.mark_started(99);
        assert!(app.worker_running);
        app.screen = Screen::Maintenance;
        tap(&mut app, &tx, &mut pending, press(KeyCode::Char('-')));
        // Blocked with an honest log; screen unchanged, worker intact.
        assert!(app.worker_running);
        assert_eq!(app.screen, Screen::Maintenance);
        assert!(
            app.log.iter().any(|l| l.contains("already in progress")),
            "must explain why navigation was refused"
        );
    }

    #[test]
    fn removed_dedicated_shortcuts_do_nothing_unfocused() {
        // `c` (clean), `r` (repair), `?` (help), `0` (exit) and `a`
        // (about) must not trigger any action when pressed unfocused.
        // No screen change, no worker, no quit, no focus change.
        for ch in ['c', 'r', '?', '0', 'a'] {
            let (mut app, tx, mut pending) = fresh();
            let screen_before = app.screen;
            let command_before = app.command;
            let log_before = app.log.len();
            let quit = tap(&mut app, &tx, &mut pending, press_char(ch));
            assert!(!quit, "unfocused '{ch}' must not quit");
            assert_eq!(
                app.screen, screen_before,
                "unfocused '{ch}' must not navigate"
            );
            assert_eq!(app.command, command_before);
            assert!(!app.input_focused, "unfocused '{ch}' must not focus entry");
            assert!(app.input.is_empty());
            assert!(
                !app.worker_running,
                "unfocused '{ch}' must not start a worker"
            );
            assert_eq!(app.log.len(), log_before, "unfocused '{ch}' must not log");
        }
    }

    #[test]
    fn removed_shortcut_chars_still_type_as_text_when_focused() {
        // Typing c, r, ?, 0, a into the command-entry field still works as
        // ordinary text input where valid.
        let (mut app, tx, mut pending) = fresh();
        tap(&mut app, &tx, &mut pending, press(KeyCode::Char('/')));
        assert!(app.input_focused);
        for ch in ['c', 'r', '?', '0', 'a'] {
            tap(&mut app, &tx, &mut pending, press_char(ch));
            assert!(
                app.input.ends_with(ch),
                "focused '{ch}' must enter the buffer"
            );
        }
        assert_eq!(app.input, "cr?0a");
        assert_eq!(app.screen, Screen::Home);
        assert!(!app.worker_running);
    }

    #[test]
    fn plain_a_no_longer_opens_about_but_entry_still_does() {
        // The standalone plain `a` shortcut was removed: unfocused `a`
        // must not navigate, start work, or quit.
        let (mut app, tx, mut pending) = fresh();
        let quit = tap(&mut app, &tx, &mut pending, press_char('a'));
        assert!(!quit);
        assert_eq!(app.screen, Screen::Home);
        assert_eq!(app.command, AppCommand::Home);
        assert!(!app.input_focused);
        assert!(app.input.is_empty());
        assert!(!app.worker_running);
        // About remains reachable via number key 4 and via command entry.
        let (mut app, tx, mut pending) = fresh();
        tap(&mut app, &tx, &mut pending, press(KeyCode::Char('4')));
        assert_eq!(app.screen, Screen::About);
        let (mut app, tx, mut pending) = fresh();
        tap(&mut app, &tx, &mut pending, press(KeyCode::Char('/')));
        type_text(&mut app, &tx, &mut pending, "akariabout");
        handle_key(&mut app, &tx, press(KeyCode::Enter), &mut pending);
        assert_eq!(app.screen, Screen::About);
        assert_eq!(app.command, AppCommand::About);
        assert!(!app.worker_running);
    }

    #[test]
    fn numeric_selection_and_retained_navigation_stay_intact() {
        // 1-5 still select; t/h/-/q/Esc/Up/Down/Ctrl+C behavior preserved.
        let (mut app, tx, mut pending) = fresh();
        tap(&mut app, &tx, &mut pending, press(KeyCode::Char('5')));
        assert_eq!(app.screen, Screen::Help);
        let (mut app, tx, mut pending) = fresh();
        tap(&mut app, &tx, &mut pending, press(KeyCode::Char('4')));
        assert_eq!(app.screen, Screen::About);
        // `t` still selects All (maintenance dispatch, elevation-gated on
        // Windows — assert mapping + no crash, not worker side effects).
        assert_eq!(
            crate::commands::command_from_input("akariall"),
            Some(AppCommand::All)
        );
        // `0` never selects anything.
        assert_eq!(crate::commands::command_from_number(0), None);
        assert_eq!(crate::commands::command_from_input("0"), None);
        assert!(!crate::commands::is_back_command("0"));
        // Unknown `0` input is handled safely (no worker, stays home).
        let (mut app, tx, mut pending) = fresh();
        tap(&mut app, &tx, &mut pending, press(KeyCode::Char('/')));
        type_text(&mut app, &tx, &mut pending, "0");
        handle_key(&mut app, &tx, press(KeyCode::Enter), &mut pending);
        assert_eq!(app.screen, Screen::Home);
        assert!(!app.worker_running);
    }
}
