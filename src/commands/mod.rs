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
pub fn from_token(token: &str) -> Option<AppCommand> {
    match token {
        "akari" | "home" | "" => Some(AppCommand::Home),
        "akariclean" | "clean" | "akariclean.exe" => Some(AppCommand::Clean),
        "akarihealth" | "health" | "repair" => Some(AppCommand::Health),
        "akariall" | "all" => Some(AppCommand::All),
        "akariabout" | "about" => Some(AppCommand::About),
        "akarihelp" | "help" | "--help" | "-h" | "/?" => Some(AppCommand::Help),
        _ => None,
    }
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
}
