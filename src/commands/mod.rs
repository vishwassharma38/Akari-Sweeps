//! Public command registry and executable-name dispatch.
//!
//! The six public names share one binary. Dispatch inspects the executable
//! file stem first (so `akariclean.exe` works with no arguments), then
//! falls back to the first CLI argument (so `akari.exe clean` also works).

pub mod all;
pub mod clean;
pub mod health;
pub mod runner;

/// The six public entry points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppCommand {
    /// `akari` — interactive TUI, no maintenance on start.
    Home,
    /// `akariclean` — cleanup phase only.
    Clean,
    /// `akarihealth` — system-image repair phase only.
    Health,
    /// `akariall` — cleanup followed by repair.
    All,
    /// `akariabout` — About screen, no maintenance.
    About,
    /// `akarihelp` — Help screen, no maintenance.
    Help,
}

impl AppCommand {
    pub fn public_name(self) -> &'static str {
        match self {
            AppCommand::Home => "akari",
            AppCommand::Clean => "akariclean",
            AppCommand::Health => "akarihealth",
            AppCommand::All => "akariall",
            AppCommand::About => "akariabout",
            AppCommand::Help => "akarihelp",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            AppCommand::Home => "Launch the Akari-Sweeps interactive terminal interface.",
            AppCommand::Clean => "Run the cleanup phase only.",
            AppCommand::Health => "Run the Windows system-image repair phase only.",
            AppCommand::All => "Run cleanup followed by system-image repair.",
            AppCommand::About => "Display application information.",
            AppCommand::Help => "Display usage instructions and available commands.",
        }
    }

    /// Whether this entry point starts maintenance without further input.
    pub fn starts_maintenance(self) -> bool {
        matches!(
            self,
            AppCommand::Clean | AppCommand::Health | AppCommand::All
        )
    }

    #[allow(dead_code)]
    pub fn wants_cleanup(self) -> bool {
        matches!(self, AppCommand::Clean | AppCommand::All)
    }

    #[allow(dead_code)]
    pub fn wants_repair(self) -> bool {
        matches!(self, AppCommand::Health | AppCommand::All)
    }
}

/// All six commands in catalogue order.
pub const COMMAND_CATALOGUE: [AppCommand; 6] = [
    AppCommand::Home,
    AppCommand::Clean,
    AppCommand::Health,
    AppCommand::All,
    AppCommand::About,
    AppCommand::Help,
];

/// The five TUI-selectable commands, in number-key order (`1`–`5`).
/// `akari` launches the interface and is intentionally not selectable
/// inside its own UI.
pub const SELECTABLE_COMMANDS: [AppCommand; 5] = [
    AppCommand::Clean,
    AppCommand::Health,
    AppCommand::All,
    AppCommand::About,
    AppCommand::Help,
];

/// Map a TUI number key (`1`–`5`) to its command. Shared by number-key
/// selection; text entry resolves through [`command_from_input`] and both
/// converge on [`AppCommand`], a single dispatch path.
pub fn command_from_number(n: u8) -> Option<AppCommand> {
    match n {
        1 => Some(AppCommand::Clean),
        2 => Some(AppCommand::Health),
        3 => Some(AppCommand::All),
        4 => Some(AppCommand::About),
        5 => Some(AppCommand::Help),
        _ => None,
    }
}

/// Map raw command-entry text to a command: trims surrounding whitespace
/// and matches case-insensitively. Empty or unknown input yields `None`
/// (the caller shows a hint instead of starting anything).
pub fn command_from_input(raw: &str) -> Option<AppCommand> {
    let t = normalize_token(raw);
    if t.is_empty() {
        return None;
    }
    from_token(&t)
}

/// Maintenance phase identity. Cleanup and repair are distinct phases;
/// illegal transitions are rejected by construction (see [`PhaseGate`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Cleanup,
    Repair,
}

impl AppCommand {
    /// Phases this command is allowed to execute, in order.
    /// `Clean` → cleanup only, `Health` → repair only,
    /// `All` → cleanup then repair. Non-maintenance commands allow none.
    pub fn allowed_phases(self) -> &'static [Phase] {
        match self {
            AppCommand::Clean => &[Phase::Cleanup],
            AppCommand::Health => &[Phase::Repair],
            AppCommand::All => &[Phase::Cleanup, Phase::Repair],
            AppCommand::Home | AppCommand::About | AppCommand::Help => &[],
        }
    }
}

/// Capability token authorizing the cleanup phase. Only [`PhaseGate`]
/// can mint one, and only for commands whose [`AppCommand::allowed_phases`]
/// include [`Phase::Cleanup`]. Phase runners take the matching token, so
/// code holding only a `CleanupGate` cannot invoke the repair phase —
/// an illegal transition fails to compile.
#[derive(Debug)]
pub struct CleanupGate {
    _private: (),
}

/// Capability token authorizing the repair phase. See [`CleanupGate`].
#[derive(Debug)]
pub struct RepairGate {
    _private: (),
}

/// Mints phase gates for exactly one command invocation.
pub struct PhaseGate;

impl PhaseGate {
    /// Returns the gates the command may use, in phase order.
    /// `Clean` → cleanup only; `Health` → repair only;
    /// `All` → both; anything else → neither.
    pub fn for_command(cmd: AppCommand) -> (Option<CleanupGate>, Option<RepairGate>) {
        let phases = cmd.allowed_phases();
        let cleanup = phases
            .contains(&Phase::Cleanup)
            .then(|| CleanupGate { _private: () });
        let repair = phases
            .contains(&Phase::Repair)
            .then(|| RepairGate { _private: () });
        (cleanup, repair)
    }
}

/// Normalize an executable stem or CLI token: lowercase, strip `.exe`,
/// strip directory components.
pub fn normalize_token(raw: &str) -> String {
    let mut s = raw.trim().to_lowercase();
    // Strip directories: both separators.
    if let Some(pos) = s.rfind(['/', '\\']) {
        s = s[pos + 1..].to_string();
    }
    if let Some(stripped) = s.strip_suffix(".exe") {
        s = stripped.to_string();
    }
    s
}

/// Map one normalized token to a command. Accepts full public names
/// (`akariclean`), bare phase words (`clean`), and help flags.
///
/// Main-menu navigation: `back`, `akariback`, and `-` all resolve to
/// [`AppCommand::Home`]. Matching is done on the caller-normalized
/// (trimmed, lower-cased) token, so `BACK`, `Akariback`, and
/// `  back  ` behave identically. A bare `-` never collides with
/// `--help`/`-h`, which remain [`AppCommand::Help`].
/// Numeric keys (`1`–`5`) are handled separately via
/// [`command_from_number`] and never mean "back": `1` always selects
/// cleanup, never returns to the menu.
pub fn from_token(token: &str) -> Option<AppCommand> {
    match token {
        "akari" | "home" | "" => Some(AppCommand::Home),
        // Dedicated, unambiguous return-to-menu commands. `1` is
        // intentionally NOT a back command.
        "back" | "akariback" | "-" => Some(AppCommand::Home),
        "akariclean" | "clean" | "akariclean.exe" => Some(AppCommand::Clean),
        "akarihealth" | "health" | "repair" => Some(AppCommand::Health),
        "akariall" | "all" => Some(AppCommand::All),
        "akariabout" | "about" => Some(AppCommand::About),
        "akarihelp" | "help" | "--help" | "-h" | "/?" | "?" => Some(AppCommand::Help),
        _ => None,
    }
}

/// True when the raw input text is a dedicated back-to-menu command
/// (`back`, `akariback`, `-`), matched case-insensitively with
/// surrounding whitespace ignored.
pub fn is_back_command(raw: &str) -> bool {
    matches!(normalize_token(raw).as_str(), "back" | "akariback" | "-")
}

/// Resolve the requested command from `(exe_path, cli_args)`.
///
/// 1. If the executable stem itself is a public alias (e.g.
///    `akariclean.exe`), it wins — even with extra args.
/// 2. Otherwise the first CLI argument is tried (`akari clean`).
/// 3. Default: `Home`.
pub fn resolve_command(exe_path: &str, args: &[String]) -> AppCommand {
    let stem = normalize_token(exe_path);
    // Only honor exe-name dispatch for real public aliases, not bare words.
    if matches!(
        stem.as_str(),
        "akariclean" | "akarihealth" | "akariall" | "akariabout" | "akarihelp"
    ) {
        if let Some(cmd) = from_token(&stem) {
            return cmd;
        }
    }
    if stem == "akari" || stem == "akari-sweeps" || stem == "akari_sweeps" {
        if let Some(first) = args.first() {
            let t = normalize_token(first);
            if let Some(cmd) = from_token(&t) {
                // `akari` with no meaningful arg stays Home; `akari ""` too.
                if !(t.is_empty() && cmd == AppCommand::Home) {
                    return cmd;
                }
                if !t.is_empty() {
                    return cmd;
                }
            }
            // Unknown first arg to `akari`: stay Home (TUI shows error hint).
            return AppCommand::Home;
        }
        return AppCommand::Home;
    }
    // Unknown exe name (e.g. dev `cargo run` binary): try first arg.
    if let Some(first) = args.first() {
        if let Some(cmd) = from_token(&normalize_token(first)) {
            return cmd;
        }
    }
    AppCommand::Home
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_name_dispatch_wins() {
        assert_eq!(resolve_command("akariclean.exe", &[]), AppCommand::Clean);
        assert_eq!(
            resolve_command("C:\\tools\\akarihealth.exe", &[]),
            AppCommand::Health
        );
        assert_eq!(resolve_command("/usr/bin/akariall", &[]), AppCommand::All);
        assert_eq!(
            resolve_command("akariabout.exe", &["clean".into()]),
            AppCommand::About
        );
    }

    #[test]
    fn arg_dispatch_for_shared_binary() {
        let exe = "akari.exe";
        assert_eq!(resolve_command(exe, &[]), AppCommand::Home);
        assert_eq!(
            resolve_command(exe, &["clean".to_string()]),
            AppCommand::Clean
        );
        assert_eq!(
            resolve_command(exe, &["akariclean".to_string()]),
            AppCommand::Clean
        );
        assert_eq!(
            resolve_command(exe, &["health".to_string()]),
            AppCommand::Health
        );
        assert_eq!(resolve_command(exe, &["all".to_string()]), AppCommand::All);
        assert_eq!(
            resolve_command(exe, &["about".to_string()]),
            AppCommand::About
        );
        assert_eq!(
            resolve_command(exe, &["--help".to_string()]),
            AppCommand::Help
        );
    }

    #[test]
    fn phase_selection_is_isolated() {
        assert!(AppCommand::Clean.wants_cleanup() && !AppCommand::Clean.wants_repair());
        assert!(!AppCommand::Health.wants_cleanup() && AppCommand::Health.wants_repair());
        assert!(AppCommand::All.wants_cleanup() && AppCommand::All.wants_repair());
        assert!(!AppCommand::Home.starts_maintenance());
        assert!(!AppCommand::About.starts_maintenance());
        assert!(!AppCommand::Help.starts_maintenance());
    }

    #[test]
    fn selectable_list_has_five_and_excludes_launcher() {
        assert_eq!(SELECTABLE_COMMANDS.len(), 5);
        assert!(!SELECTABLE_COMMANDS.contains(&AppCommand::Home));
        let names: Vec<_> = SELECTABLE_COMMANDS
            .iter()
            .map(|c| c.public_name())
            .collect();
        assert_eq!(
            names,
            vec![
                "akariclean",
                "akarihealth",
                "akariall",
                "akariabout",
                "akarihelp"
            ]
        );
    }

    #[test]
    fn number_keys_map_to_selectable_commands() {
        for (i, cmd) in SELECTABLE_COMMANDS.iter().enumerate() {
            assert_eq!(command_from_number((i + 1) as u8), Some(*cmd));
        }
        assert_eq!(command_from_number(0), None);
        assert_eq!(command_from_number(6), None);
        // No number ever selects the launcher itself.
        assert!(!(1..=9).any(|n| command_from_number(n) == Some(AppCommand::Home)));
    }

    #[test]
    fn text_entry_agrees_with_number_selection() {
        // Every selectable command: typed name == number-key dispatch.
        for (i, cmd) in SELECTABLE_COMMANDS.iter().enumerate() {
            let typed = command_from_input(cmd.public_name());
            assert_eq!(typed, Some(*cmd));
            assert_eq!(typed, command_from_number((i + 1) as u8));
        }
        // Normalization: whitespace trimmed, case-insensitive.
        assert_eq!(
            command_from_input("  AkariClean  "),
            Some(AppCommand::Clean)
        );
        assert_eq!(command_from_input("AKARIHELP"), Some(AppCommand::Help));
        // Empty / unknown input dispatches to nothing.
        assert_eq!(command_from_input(""), None);
        assert_eq!(command_from_input("   "), None);
        assert_eq!(command_from_input("akaricleen"), None);
        assert_eq!(command_from_input("format c:"), None);
    }

    #[test]
    fn allowed_phases_isolate_cleanup_and_repair() {
        assert_eq!(AppCommand::Clean.allowed_phases(), &[Phase::Cleanup]);
        assert_eq!(AppCommand::Health.allowed_phases(), &[Phase::Repair]);
        assert_eq!(
            AppCommand::All.allowed_phases(),
            &[Phase::Cleanup, Phase::Repair]
        );
        assert!(AppCommand::Home.allowed_phases().is_empty());
        assert!(AppCommand::About.allowed_phases().is_empty());
        assert!(AppCommand::Help.allowed_phases().is_empty());
    }

    #[test]
    fn gates_mint_only_permitted_phases() {
        let (c, r) = PhaseGate::for_command(AppCommand::Clean);
        assert!(c.is_some() && r.is_none());
        let (c, r) = PhaseGate::for_command(AppCommand::Health);
        assert!(c.is_none() && r.is_some());
        let (c, r) = PhaseGate::for_command(AppCommand::All);
        assert!(c.is_some() && r.is_some());
        for cmd in [AppCommand::Home, AppCommand::About, AppCommand::Help] {
            let (c, r) = PhaseGate::for_command(cmd);
            assert!(c.is_none() && r.is_none());
        }
    }

    #[test]
    fn catalogue_has_all_six() {
        assert_eq!(COMMAND_CATALOGUE.len(), 6);
        let names: Vec<_> = COMMAND_CATALOGUE.iter().map(|c| c.public_name()).collect();
        assert_eq!(
            names,
            vec![
                "akari",
                "akariclean",
                "akarihealth",
                "akariall",
                "akariabout",
                "akarihelp"
            ]
        );
    }

    #[test]
    fn back_commands_return_to_main_menu() {
        for raw in ["back", "akariback", "-"] {
            assert_eq!(
                command_from_input(raw),
                Some(AppCommand::Home),
                "{raw} must return to main menu"
            );
            assert!(is_back_command(raw), "{raw} must be a back command");
        }
    }

    #[test]
    fn back_matching_is_case_insensitive_and_trims_whitespace() {
        for raw in [
            "BACK",
            "Back",
            "Akariback",
            "AKARIBACK",
            "  back  ",
            "  AKARIBACK\t",
            "  -  ",
        ] {
            assert_eq!(
                command_from_input(raw),
                Some(AppCommand::Home),
                "{raw:?} must return home"
            );
            assert!(is_back_command(raw));
        }
        // Non-back commands are not back commands.
        for raw in ["akariclean", "clean", "1", "", "   ", "format c:"] {
            assert!(!is_back_command(raw), "{raw:?} must not be back");
        }
    }

    #[test]
    fn dash_does_not_collide_with_help_flags() {
        assert_eq!(command_from_input("-"), Some(AppCommand::Home));
        assert_eq!(command_from_input("--help"), Some(AppCommand::Help));
        assert_eq!(command_from_input("-h"), Some(AppCommand::Help));
        assert_eq!(command_from_input("/?"), Some(AppCommand::Help));
        assert!(!is_back_command("--help"));
        assert!(!is_back_command("-h"));
    }

    #[test]
    fn one_never_means_exit_it_selects_cleanup() {
        assert_eq!(command_from_number(1), Some(AppCommand::Clean));
        assert_ne!(command_from_number(1), Some(AppCommand::Home));
        assert_eq!(command_from_input("1"), None, "typed 1 is not a command");
        assert!(!is_back_command("1"));
    }

    #[test]
    fn numeric_shortcuts_still_launch_correct_operations() {
        assert_eq!(command_from_number(1), Some(AppCommand::Clean));
        assert_eq!(command_from_number(2), Some(AppCommand::Health));
        assert_eq!(command_from_number(3), Some(AppCommand::All));
        assert_eq!(command_from_number(4), Some(AppCommand::About));
        assert_eq!(command_from_number(5), Some(AppCommand::Help));
    }
}
