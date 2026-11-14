//! GitHub Release-based self-updater.
//!
//! The updater is deliberately separate from the TUI command registry: the
//! six public commands and their `AppCommand` dispatch stay untouched, while
//! `akari update` / `akari update --check` are handled at the CLI entry point
//! before the alternate screen or elevation logic runs.
//!
//! Responsibilities are split by module:
//! - [`github`] — public GitHub Releases API and exact asset selection.
//! - [`version`] — Cargo-sourced installed version and semantic comparison.
//! - [`download`] — checksum verification and hardened ZIP extraction.
//! - [`install`] — installation-layout discovery and staging preparation.
//! - [`powershell`] — embedded helper and safe argument construction.
//! - [`cache`] — the 24-hour automatic-check timestamp.

pub mod cache;
pub mod download;
pub mod github;
pub mod install;
pub mod powershell;
pub mod version;

use semver::Version;
use std::fmt;

pub type Result<T> = std::result::Result<T, UpdateError>;

/// Every updater failure is converted into a readable message; no updater
/// code path relies on panics or exposes raw internal state to the user.
#[derive(Debug)]
pub enum UpdateError {
    /// DNS, TLS, connection, timeout, and interrupted-transfer failures.
    Network(String),
    /// A successful HTTP exchange that did not contain a usable release.
    Api(String),
    /// GitHub rejected the request because the public rate limit was hit.
    RateLimited,
    /// Missing or malformed release metadata/asset information.
    InvalidRelease(String),
    /// A tag that is not valid semantic versioning.
    InvalidVersion(String),
    /// The release did not contain the exact required asset.
    MissingAsset(String),
    /// The archive or its published checksum disagreed or was malformed.
    Checksum(String),
    /// Unsafe, corrupt, or unsupported archive contents.
    Archive(String),
    /// Local filesystem problems (workspace, staged files, logs).
    Io(String),
    /// The running executable is not part of a complete installation.
    UnsupportedInstallation(String),
    /// Installation is only implemented for Windows.
    UnsupportedPlatform,
    /// The PowerShell helper could not be written or launched.
    Helper(String),
}

impl UpdateError {
    /// A short, actionable hint printed after the technical message.
    pub fn user_hint(&self) -> &'static str {
        match self {
            UpdateError::RateLimited => {
                "GitHub's API rate limit was reached. Wait a few minutes and try again."
            }
            UpdateError::Network(_) => "Check your internet connection and try again.",
            UpdateError::UnsupportedPlatform => {
                "Akari-Sweeps updates can only be installed on Windows."
            }
            UpdateError::UnsupportedInstallation(_) => {
                "Run Akari-Sweeps from its installed directory, or reinstall it with install.ps1."
            }
            UpdateError::Helper(_) => {
                "Windows PowerShell could not be started. No installed files were changed."
            }
            _ => "The update was not installed; the existing installation was left unchanged.",
        }
    }
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateError::Network(message) => write!(f, "network error: {message}"),
            UpdateError::Api(message) => write!(f, "GitHub API error: {message}"),
            UpdateError::RateLimited => write!(f, "GitHub's API rate limit was reached"),
            UpdateError::InvalidRelease(message) => write!(f, "invalid release: {message}"),
            UpdateError::InvalidVersion(message) => {
                write!(f, "invalid release version: {message}")
            }
            UpdateError::MissingAsset(message) => {
                write!(f, "incomplete release: {message}")
            }
            UpdateError::Checksum(message) => {
                write!(f, "SHA-256 verification failed: {message}")
            }
            UpdateError::Archive(message) => write!(f, "release package rejected: {message}"),
            UpdateError::Io(message) => write!(f, "file error: {message}"),
            UpdateError::UnsupportedInstallation(message) => write!(f, "{message}"),
            UpdateError::UnsupportedPlatform => {
                write!(f, "updates can only be installed on Windows")
            }
            UpdateError::Helper(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for UpdateError {}

/// Notification payload delivered to the TUI by the background check. It is
/// intentionally small: the automatic path never downloads or installs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateNotice {
    pub installed_version: String,
    pub available_version: String,
    pub release_url: String,
}

/// Result of comparing the installed application with the latest release.
#[derive(Debug, Clone)]
pub struct CheckReport {
    pub installed: Version,
    pub latest: Version,
    pub availability: version::Availability,
    pub release: github::Release,
}

impl CheckReport {
    pub fn installed_version(&self) -> String {
        self.installed.to_string()
    }

    pub fn latest_version(&self) -> String {
        self.latest.to_string()
    }
}

/// Installed-version line used by `akari --version`.
pub fn version_line() -> String {
    format!("Akari-Sweeps {}", version::CURRENT_VERSION)
}

/// True only for the primary launcher names. Automatic update checks and the
/// updater CLI are intentionally unavailable through maintenance aliases.
pub fn is_primary_launch(exe: &str) -> bool {
    matches!(
        crate::commands::normalize_token(exe).as_str(),
        "akari" | "akari-sweeps" | "akari_sweeps"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EarlyCli {
    Version,
    Update { check_only: bool },
}

/// Parse only the updater-specific arguments for the primary launcher.
///
/// `Ok(None)` means the invocation is an ordinary command and must continue
/// to the existing dispatcher/TUI. `Err` is a usage error and exits with a
/// nonzero code without starting the TUI.
fn parse_early_cli(exe: &str, args: &[String]) -> std::result::Result<Option<EarlyCli>, String> {
    if !is_primary_launch(exe) {
        return Ok(None);
    }
    let Some(first) = args.first() else {
        return Ok(None);
    };
    if first.eq_ignore_ascii_case("--version") || first == "-V" {
        if args.len() != 1 {
            return Err("usage: akari --version".to_string());
        }
        return Ok(Some(EarlyCli::Version));
    }
    if !first.eq_ignore_ascii_case("update") {
        return Ok(None);
    }
    match args.len() {
        1 => Ok(Some(EarlyCli::Update { check_only: false })),
        2 if args[1].eq_ignore_ascii_case("--check") => {
            Ok(Some(EarlyCli::Update { check_only: true }))
        }
        2 => Err(format!(
            "unknown option '{}' for 'akari update'.\nusage: akari update [--check]",
            args[1]
        )),
        _ => Err("usage: akari update [--check]".to_string()),
    }
}

/// Handle `akari --version`, `akari update`, and `akari update --check` before
/// elevation and terminal ownership. Returns `None` when the invocation is not
/// an updater command.
pub fn handle_early_cli(exe: &str, args: &[String]) -> Option<i32> {
    match parse_early_cli(exe, args) {
        Ok(None) => None,
        Ok(Some(EarlyCli::Version)) => {
            println!("{}", version_line());
            Some(0)
        }
        Ok(Some(EarlyCli::Update { check_only })) => Some(run_update_cli(check_only)),
        Err(message) => {
            eprintln!("{message}");
            Some(2)
        }
    }
}

fn run_update_cli(check_only: bool) -> i32 {
    let report = match check_latest() {
        Ok(report) => report,
        Err(error) => {
            print_update_error(&error);
            return 1;
        }
    };

    for line in boxed_lines("AKARI-SWEEPS UPDATE", &report_lines(&report, check_only)) {
        println!("{line}");
    }

    if check_only {
        return 0;
    }
    if report.availability != version::Availability::UpdateAvailable {
        println!("No update is needed.");
        return 0;
    }

    match install_available(&report) {
        Ok(()) => 0,
        Err(error) => {
            print_update_error(&error);
            1
        }
    }
}

fn print_update_error(error: &UpdateError) {
    eprintln!("Akari-Sweeps update failed: {error}");
    eprintln!("{}", error.user_hint());
}

/// Query the latest published release and compare it with the installed
/// version. Manual commands always call this directly, bypassing the cache.
pub fn check_latest() -> Result<CheckReport> {
    check_latest_with(github::fetch_latest_release)
}

/// Testable discovery/comparison split: the supplied closure replaces only
/// the network call.
pub fn check_latest_with<F>(fetch: F) -> Result<CheckReport>
where
    F: FnOnce() -> Result<github::Release>,
{
    let release = fetch()?;
    let latest = release.version()?;
    let installed = version::installed_version();
    Ok(CheckReport {
        availability: version::availability(&installed, &latest),
        installed,
        latest,
        release,
    })
}

/// First meaningful release-note line, shortened so the CLI box stays
/// readable. The full notes remain available at the release page.
fn release_summary(body: Option<&str>) -> Option<String> {
    let line = body?.lines().map(str::trim).find(|line| !line.is_empty())?;
    let mut summary: String = line.chars().take(100).collect();
    if line.chars().count() > 100 {
        summary.push_str("...");
    }
    Some(summary)
}

/// Text body used by both `akari update` and `akari update --check`.
pub fn report_lines(report: &CheckReport, check_only: bool) -> Vec<String> {
    let release_url = if report.release.html_url.trim().is_empty() {
        "unavailable"
    } else {
        report.release.html_url.as_str()
    };
    let mut lines = vec![
        format!("Installed Version : {}", report.installed_version()),
        format!("Latest Version    : {}", report.latest_version()),
        format!("Release Page      : {release_url}"),
    ];
    if let Some(summary) = release_summary(report.release.body.as_deref()) {
        lines.push(format!("Notes             : {summary}"));
    }
    lines.push(String::new());
    match report.availability {
        version::Availability::UpdateAvailable => {
            lines.push("A new version is available!".to_string());
            if check_only {
                lines.push(String::new());
                lines.push("Run: akari update".to_string());
            }
        }
        version::Availability::UpToDate => {
            lines.push("Akari-Sweeps is up to date.".to_string());
            if check_only {
                lines.push(String::new());
                lines.push("Nothing to install.".to_string());
            }
        }
        version::Availability::NewerInstalled => {
            lines.push("The installed version is newer than the latest release.".to_string());
            if check_only {
                lines.push(String::new());
                lines.push("No update needed.".to_string());
            }
        }
    }
    lines
}

/// Render an ASCII box so output is safe in legacy Windows consoles as well
/// as modern terminals. Box drawing characters are intentionally avoided.
pub fn boxed_lines(title: &str, body: &[String]) -> Vec<String> {
    let width = body
        .iter()
        .map(|line| line.chars().count())
        .chain(std::iter::once(title.chars().count()))
        .max()
        .unwrap_or(0)
        .max(40);
    let border = format!("+{}+", "-".repeat(width + 2));
    let mut lines = vec![
        border.clone(),
        format!("| {:^width$} |", title),
        border.clone(),
    ];
    for line in body {
        lines.push(format!("| {line:<width$} |"));
    }
    lines.push(border);
    lines
}

/// Download, verify, stage, and launch the helper. This function returns as
/// soon as the helper process has been created; the Rust process must exit so
/// Windows can release the running executable.
fn install_available(report: &CheckReport) -> Result<()> {
    if !cfg!(windows) {
        return Err(UpdateError::UnsupportedPlatform);
    }
    let assets = report.release.windows_assets()?;
    let layout = install::InstallLayout::discover()?;
    let layout_report = layout.validate()?;

    println!("Installation directory: {}", layout.directory.display());
    if !layout_report.missing_aliases.is_empty() {
        println!(
            "Missing aliases will be recreated: {}",
            layout_report.missing_aliases.join(", ")
        );
    }

    let workspace = download::UpdateWorkspace::create()?;
    let result = (|| {
        println!("Downloading {}…", github::WINDOWS_PACKAGE);
        let verified = download::download_release(&assets, &workspace)?;
        println!("SHA-256 verified: {}", verified.sha256);

        println!("Extracting and validating the release package…");
        let launch = install::prepare_installation(&workspace, &layout, &verified.zip_path)?;

        println!("Starting the PowerShell update helper…");
        launch.spawn()?;
        println!();
        println!("The update is running in a separate PowerShell process.");
        println!(
            "Do not start Akari-Sweeps until it finishes; it waits for this process \
             and any other running alias to exit."
        );
        println!("Progress and errors: {}", launch.log_path.display());
        Ok(())
    })();

    if result.is_err() {
        workspace.cleanup();
    }
    result
}

/// Automatic check used only by the primary launcher. The cache timestamp is
/// recorded before the network request so a failed attempt also respects the
/// 24-hour interval. Failures are deliberately silent: they must never block
/// or interrupt normal startup.
pub fn automatic_check(cache: &cache::CacheStore) -> Option<UpdateNotice> {
    automatic_check_with(cache, cache::now_unix_secs(), github::fetch_latest_release)
}

/// Testable automatic-check policy. `fetch` is only called when the cache
/// allows a check; an invalid/corrupt cache behaves like a missing one.
pub fn automatic_check_with<F>(
    cache_store: &cache::CacheStore,
    now_unix_secs: u64,
    fetch: F,
) -> Option<UpdateNotice>
where
    F: FnOnce() -> Result<github::Release>,
{
    if !cache_store.should_check(now_unix_secs) {
        return None;
    }
    cache_store.record_check_best_effort(now_unix_secs);
    notice_for_release(&fetch().ok()?)
}

/// Compare one release with the installed version and build the notification.
pub fn notice_for_release(release: &github::Release) -> Option<UpdateNotice> {
    let latest = release.version().ok()?;
    if !version::is_newer(&version::installed_version(), &latest) {
        return None;
    }
    Some(UpdateNotice {
        installed_version: version::CURRENT_VERSION.to_string(),
        available_version: latest.to_string(),
        release_url: release.html_url.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn release(tag: &str) -> github::Release {
        github::Release {
            tag_name: tag.to_string(),
            html_url: format!("https://github.com/vishwassharma38/Akari-Sweeps/releases/tag/{tag}"),
            body: Some("Release notes".to_string()),
            assets: vec![github::ReleaseAsset {
                name: github::WINDOWS_PACKAGE.to_string(),
                browser_download_url: format!(
                    "https://github.com/vishwassharma38/Akari-Sweeps/releases/download/{tag}/{}",
                    github::WINDOWS_PACKAGE
                ),
                digest: Some(
                    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                        .to_string(),
                ),
            }],
        }
    }

    fn temp_cache(label: &str) -> cache::CacheStore {
        let root = std::env::temp_dir().join(format!(
            "akari-update-mod-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        cache::CacheStore::new(root.join("update-check.json"))
    }

    #[test]
    fn primary_launch_names_are_exact_and_aliases_are_excluded() {
        for exe in [
            "akari",
            "akari.exe",
            "C:\\Tools\\akari-sweeps.exe",
            "akari_sweeps",
        ] {
            assert!(is_primary_launch(exe), "{exe}");
        }
        for exe in [
            "akariclean.exe",
            "akarihealth.exe",
            "akariall.exe",
            "akariabout.exe",
            "akarihelp.exe",
            "unrelated.exe",
        ] {
            assert!(!is_primary_launch(exe), "{exe}");
        }
    }

    #[test]
    fn update_arguments_are_parsed_only_for_the_primary_launcher() {
        let args = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse_early_cli("akari.exe", &args(&["update"])).unwrap(),
            Some(EarlyCli::Update { check_only: false })
        );
        assert_eq!(
            parse_early_cli("akari.exe", &args(&["UPDATE", "--CHECK"])).unwrap(),
            Some(EarlyCli::Update { check_only: true })
        );
        assert_eq!(
            parse_early_cli("akari-sweeps.exe", &args(&["--version"])).unwrap(),
            Some(EarlyCli::Version)
        );
        // Maintenance aliases never gain updater commands.
        assert_eq!(
            parse_early_cli("akariclean.exe", &args(&["update"])).unwrap(),
            None
        );
        // Ordinary navigation is untouched.
        assert_eq!(
            parse_early_cli("akari.exe", &args(&["clean"])).unwrap(),
            None
        );
        assert_eq!(parse_early_cli("akari.exe", &[]).unwrap(), None);
    }

    #[test]
    fn malformed_update_arguments_are_usage_errors() {
        let args = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for items in [
            vec!["update", "--check", "extra"],
            vec!["update", "now"],
            vec!["--version", "extra"],
        ] {
            assert!(
                parse_early_cli("akari.exe", &args(&items)).is_err(),
                "{items:?} must be rejected"
            );
        }
    }

    #[test]
    fn version_line_uses_the_cargo_version() {
        assert_eq!(
            version_line(),
            format!("Akari-Sweeps {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn update_available_report_includes_versions_and_run_hint() {
        let report = check_latest_with(|| Ok(release("v999.0.0"))).unwrap();
        assert_eq!(report.availability, version::Availability::UpdateAvailable);
        let lines = report_lines(&report, true).join("\n");
        assert!(lines.contains(&format!(
            "Installed Version : {}",
            env!("CARGO_PKG_VERSION")
        )));
        assert!(lines.contains("Latest Version    : 999.0.0"));
        assert!(lines.contains("A new version is available!"));
        assert!(lines.contains("Run: akari update"));
        assert!(lines.contains(&report.release.html_url));
    }

    #[test]
    fn up_to_date_report_says_so_and_never_offers_installation() {
        let report =
            check_latest_with(|| Ok(release(&format!("v{}", env!("CARGO_PKG_VERSION"))))).unwrap();
        assert_eq!(report.availability, version::Availability::UpToDate);
        let lines = report_lines(&report, true).join("\n");
        assert!(lines.contains("Akari-Sweeps is up to date."));
        assert!(lines.contains("Nothing to install."));
        assert!(!lines.contains("Run: akari update"));
    }

    #[test]
    fn newer_installed_report_is_not_reported_as_an_update() {
        let report = check_latest_with(|| Ok(release("v0.0.0"))).unwrap();
        assert_eq!(report.availability, version::Availability::NewerInstalled);
        let lines = report_lines(&report, true).join("\n");
        assert!(lines.contains("newer than the latest release"));
        assert!(!lines.contains("A new version is available!"));
    }

    #[test]
    fn malformed_release_metadata_is_rejected_before_install() {
        let mut release = release("v999.0.0");
        release.tag_name = "not-a-version".to_string();
        assert!(check_latest_with(|| Ok(release)).is_err());
    }

    #[test]
    fn automatic_check_returns_a_notice_for_a_newer_release() {
        let cache = temp_cache("newer");
        let notice = automatic_check_with(&cache, 1_000_000, || Ok(release("v999.0.0"))).unwrap();
        assert_eq!(notice.installed_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(notice.available_version, "999.0.0");
        assert!(notice.release_url.ends_with("/v999.0.0"));
        assert_eq!(cache.last_check_unix_secs(), Some(1_000_000));
        let _ = fs::remove_dir_all(cache.path().parent().unwrap());
    }

    #[test]
    fn automatic_check_is_silent_for_current_or_older_releases() {
        for tag in [
            format!("v{}", env!("CARGO_PKG_VERSION")),
            "v0.0.0".to_string(),
        ] {
            let cache = temp_cache("no-notice");
            let calls = Cell::new(0);
            let notice = automatic_check_with(&cache, 1_000_000, || {
                calls.set(calls.get() + 1);
                Ok(release(&tag))
            });
            assert!(notice.is_none(), "{tag}");
            assert_eq!(calls.get(), 1, "{tag}");
            let _ = fs::remove_dir_all(cache.path().parent().unwrap());
        }
    }

    #[test]
    fn automatic_check_respects_the_24_hour_cache_without_calling_github() {
        let cache = temp_cache("suppressed");
        cache.record_check(1_000_000).unwrap();
        let calls = Cell::new(0);
        let notice = automatic_check_with(&cache, 1_000_000 + 60, || {
            calls.set(calls.get() + 1);
            Ok(release("v999.0.0"))
        });
        assert!(notice.is_none());
        assert_eq!(calls.get(), 0, "fresh cache must suppress the network call");
        let _ = fs::remove_dir_all(cache.path().parent().unwrap());
    }

    #[test]
    fn failed_automatic_checks_do_not_panic_and_are_rate_limited() {
        let cache = temp_cache("failure");
        let notice = automatic_check_with(&cache, 1_000_000, || {
            Err(UpdateError::Network("offline".to_string()))
        });
        assert!(notice.is_none());
        assert_eq!(
            cache.last_check_unix_secs(),
            Some(1_000_000),
            "failed attempts must still respect the interval"
        );
        assert!(!cache.should_check(1_000_000 + 60));
        let _ = fs::remove_dir_all(cache.path().parent().unwrap());
    }

    #[test]
    fn corrupted_cache_allows_one_check_then_records_successfully() {
        let cache = temp_cache("corrupt");
        fs::create_dir_all(cache.path().parent().unwrap()).unwrap();
        fs::write(cache.path(), b"not-json").unwrap();
        let notice = automatic_check_with(&cache, 1_000_000, || Ok(release("v999.0.0")));
        assert!(notice.is_some());
        assert_eq!(cache.last_check_unix_secs(), Some(1_000_000));
        let _ = fs::remove_dir_all(cache.path().parent().unwrap());
    }

    #[test]
    fn manual_check_path_does_not_consult_the_cache() {
        let cache = temp_cache("manual-bypass");
        cache.record_check(1_000_000).unwrap();
        assert!(!cache.should_check(1_000_001));
        // Manual discovery calls this function directly; it takes no cache.
        let report = check_latest_with(|| Ok(release("v999.0.0"))).unwrap();
        assert_eq!(report.availability, version::Availability::UpdateAvailable);
        let _ = fs::remove_dir_all(cache.path().parent().unwrap());
    }

    #[test]
    fn boxed_output_is_aligned_and_contains_every_line() {
        let body = vec![
            "Installed Version : 0.1.0".to_string(),
            "A new version is available!".to_string(),
        ];
        let lines = boxed_lines("AKARI-SWEEPS UPDATE", &body);
        assert!(lines.len() >= 6);
        let first_width = lines[0].chars().count();
        for line in &lines {
            assert_eq!(
                line.chars().count(),
                first_width,
                "box line must stay aligned: {line:?}"
            );
        }
        assert_eq!(lines[0].chars().next(), Some('+'));
        assert!(lines[1].contains("AKARI-SWEEPS UPDATE"));
        assert!(lines.iter().any(|line| line.contains("Installed Version")));
    }

    #[test]
    fn update_errors_have_actionable_hints() {
        assert!(
            UpdateError::RateLimited
                .user_hint()
                .to_ascii_lowercase()
                .contains("rate limit")
        );
        assert!(
            UpdateError::Network("dns".to_string())
                .user_hint()
                .to_ascii_lowercase()
                .contains("internet")
        );
        assert!(
            UpdateError::Checksum("bad".to_string())
                .to_string()
                .contains("SHA-256")
        );
        assert!(
            UpdateError::UnsupportedPlatform
                .to_string()
                .contains("Windows")
        );
    }

    #[test]
    fn workspace_helpers_are_not_exposed_to_automatic_checks() {
        // Structural guard: automatic checks only build notices and never
        // reference download/install workspaces. This test documents the
        // invariant by checking that notice construction has no filesystem
        // side effect outside the per-user cache.
        let cache = temp_cache("no-download");
        let _ = automatic_check_with(&cache, 1_000_000, || Ok(release("v999.0.0")));
        let root: PathBuf = cache.path().parent().unwrap().to_path_buf();
        let entries: Vec<_> = fs::read_dir(&root).unwrap().collect();
        assert_eq!(entries.len(), 1, "only the cache file may be created");
        let _ = fs::remove_dir_all(root);
    }
}
