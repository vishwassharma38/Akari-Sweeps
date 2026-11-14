//! Embedded PowerShell helper used to replace a running Windows
//! installation. The script is compiled into the Rust binary, written into
//! the transaction workspace, and launched with every path as a separate
//! argument. Nothing is interpolated into script source.

use super::cache;
use super::{Result, UpdateError};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

/// Windows PowerShell 5.1 is present on supported Windows installations;
/// no separately installed PowerShell 7 runtime is required.
pub const POWERSHELL_PROGRAM: &str = "powershell.exe";
pub const SCRIPT_FILE_NAME: &str = "apply_update.ps1";
pub const DEFAULT_WAIT_SECONDS: u64 = 120;

const SCRIPT: &str = include_str!("apply_update.ps1");

#[derive(Debug, Clone)]
pub struct HelperLaunch {
    pub script_path: PathBuf,
    pub install_dir: PathBuf,
    pub staging_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub backup_dir: PathBuf,
    pub log_path: PathBuf,
    pub wait_seconds: u64,
}

impl HelperLaunch {
    /// Build the complete argument list without shell-style quoting. Each
    /// untrusted path stays exactly one `OsString`; `std::process::Command`
    /// applies the Windows quoting rules when it creates the process.
    pub fn arguments(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec![
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-File".into(),
        ];
        args.push(self.script_path.as_os_str().to_os_string());
        push_value(&mut args, "-InstallDir", &self.install_dir);
        push_value(&mut args, "-StagingDir", &self.staging_dir);
        push_value(&mut args, "-ManifestPath", &self.manifest_path);
        push_value(&mut args, "-BackupDir", &self.backup_dir);
        push_value(&mut args, "-LogPath", &self.log_path);
        push_value(
            &mut args,
            "-WaitSeconds",
            &PathBuf::from(self.wait_seconds.to_string()),
        );
        args
    }

    /// Launch the helper and return immediately. The helper waits for this
    /// process (and any other running alias) to exit before replacing files.
    #[cfg(windows)]
    pub fn spawn(&self) -> Result<()> {
        use std::process::{Command, Stdio};

        let mut command = Command::new(POWERSHELL_PROGRAM);
        command.args(self.arguments());
        command.stdin(Stdio::null());
        command.spawn().map(|_| ()).map_err(|e| {
            UpdateError::Helper(format!(
                "could not launch {POWERSHELL_PROGRAM} ({e}). Windows PowerShell 5.1 \
                 must be available."
            ))
        })
    }

    #[cfg(not(windows))]
    pub fn spawn(&self) -> Result<()> {
        Err(UpdateError::UnsupportedPlatform)
    }
}

fn push_value(args: &mut Vec<OsString>, flag: &str, value: &Path) {
    args.push(flag.into());
    args.push(value.as_os_str().to_os_string());
}

/// Write the embedded helper into the transaction workspace. The script is
/// pure ASCII so Windows PowerShell 5.1's default ANSI parsing is safe.
pub fn write_script(directory: &Path) -> Result<PathBuf> {
    fs::create_dir_all(directory).map_err(|e| {
        UpdateError::Io(format!(
            "could not create the update workspace '{}': {e}",
            directory.display()
        ))
    })?;
    let path = directory.join(SCRIPT_FILE_NAME);
    fs::write(&path, SCRIPT.as_bytes()).map_err(|e| {
        UpdateError::Io(format!(
            "could not write the embedded update helper '{}': {e}",
            path.display()
        ))
    })?;
    Ok(path)
}

/// A persistent log outside the installation directory, so it survives the
/// workspace cleanup and is easy to find after a failed update.
pub fn log_path() -> PathBuf {
    cache::user_data_dir().join("update.log")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_launch() -> HelperLaunch {
        HelperLaunch {
            script_path: PathBuf::from(r"C:\Temp\akari update\apply_update.ps1"),
            install_dir: PathBuf::from(r"D:\Tools\Akari Sweeps [x] & more"),
            staging_dir: PathBuf::from(r"C:\Temp\akari update\stage"),
            manifest_path: PathBuf::from(r"C:\Temp\akari update\manifest.sha256"),
            backup_dir: PathBuf::from(r"C:\Temp\akari update\backup"),
            log_path: PathBuf::from(r"C:\Users\Test User\AppData\Local\Akari-Sweeps\update.log"),
            wait_seconds: 120,
        }
    }

    fn value_after<'a>(args: &'a [OsString], flag: &str) -> &'a OsString {
        let index = args.iter().position(|a| a == flag).unwrap();
        &args[index + 1]
    }

    #[test]
    fn helper_arguments_keep_untrusted_paths_as_separate_arguments() {
        let launch = sample_launch();
        let args = launch.arguments();
        assert_eq!(
            value_after(&args, "-InstallDir"),
            launch.install_dir.as_os_str()
        );
        assert_eq!(
            value_after(&args, "-StagingDir"),
            launch.staging_dir.as_os_str()
        );
        assert_eq!(
            value_after(&args, "-ManifestPath"),
            launch.manifest_path.as_os_str()
        );
        assert_eq!(
            value_after(&args, "-BackupDir"),
            launch.backup_dir.as_os_str()
        );
        assert_eq!(value_after(&args, "-LogPath"), launch.log_path.as_os_str());
        assert_eq!(value_after(&args, "-File"), launch.script_path.as_os_str());
        assert_eq!(value_after(&args, "-WaitSeconds"), &OsString::from("120"));
        assert_eq!(
            args.iter()
                .filter(|a| a.to_string_lossy() == "-InstallDir")
                .count(),
            1
        );
        assert!(args.iter().any(|a| a.to_string_lossy() == "-NoProfile"));
        assert!(
            args.iter()
                .any(|a| a.to_string_lossy() == "-NonInteractive")
        );
        assert!(
            !args
                .iter()
                .any(|a| a.to_string_lossy().contains("; Install-Dir")),
            "paths must never be concatenated into script text"
        );
    }

    #[test]
    fn embedded_script_is_ascii_complete_and_written_without_bom() {
        assert!(
            SCRIPT.is_ascii(),
            "PowerShell 5.1 reads BOM-less files as ANSI"
        );
        for required in [
            "WaitSeconds",
            "BackupDir",
            "Restore-Backup",
            "TestFailAfterAliasesReplace",
            "akari.exe",
            "akarihelp.exe",
        ] {
            assert!(SCRIPT.contains(required), "helper missing {required:?}");
        }
        let dir = std::env::temp_dir().join(format!(
            "akari-powershell-write-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = write_script(&dir).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert!(!bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
        assert_eq!(bytes, SCRIPT.as_bytes());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn log_path_is_per_user_and_not_inside_an_install_directory() {
        let path = log_path();
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("update.log")
        );
        assert!(!path.starts_with(r"D:\Tools\Akari"));
        assert_eq!(
            path.parent().and_then(|p| p.file_name()),
            Some(std::ffi::OsStr::new("Akari-Sweeps"))
        );
    }
}

#[cfg(all(test, target_os = "windows"))]
mod windows_tests {
    use super::*;
    use crate::update::download;
    use std::fs::{self, OpenOptions};
    use std::os::windows::fs::OpenOptionsExt;
    use std::process::{Command, Output};
    use std::time::{SystemTime, UNIX_EPOCH};

    const OLD_BINARY: &[u8] = b"MZold-installed-binary";
    const NEW_BINARY: &[u8] = b"MZnew-release-binary";

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "akari helper {label} {}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn make_install(root: &Path, aliases: &[&str]) -> PathBuf {
        let install = root.join("install dir with spaces");
        let assets = install.join("assets").join("akari").join("idle");
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("old.png"), b"old-png").unwrap();
        fs::create_dir_all(install.join("user data")).unwrap();
        fs::write(install.join("user data").join("keep.txt"), b"keep-me").unwrap();
        fs::write(install.join("settings.json"), b"{\"theme\":\"akari\"}").unwrap();
        for alias in aliases {
            fs::write(install.join(alias), OLD_BINARY).unwrap();
        }
        install
    }

    fn make_stage(workspace: &Path, files: &[(&str, &[u8])]) -> (PathBuf, PathBuf) {
        let stage = workspace.join("stage");
        fs::create_dir_all(&stage).unwrap();
        fs::write(stage.join("akari-sweeps.exe"), NEW_BINARY).unwrap();
        for (relative, data) in files {
            let path = stage.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        }
        let manifest = workspace.join("install-manifest.sha256");
        download::write_manifest(&stage, &manifest).unwrap();
        (stage, manifest)
    }

    struct Paths {
        install: PathBuf,
        stage: PathBuf,
        manifest: PathBuf,
        backup: PathBuf,
        log: PathBuf,
        workspace: PathBuf,
    }

    fn paths(root: &Path, install: PathBuf) -> Paths {
        let workspace = root.join("update workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (stage, manifest) = make_stage(
            &workspace,
            &[
                ("assets/akari/idle/new.png", b"new-png"),
                ("assets/akari/sweep/new.png", b"newer-png"),
            ],
        );
        Paths {
            install,
            stage,
            manifest,
            backup: workspace.join("backup"),
            log: root.join("logs").join("update.log"),
            workspace,
        }
    }

    fn run_helper(paths: &Paths, wait_seconds: u64, inject_failure: bool) -> Output {
        let script_dir = paths.workspace.join("helper");
        let script = write_script(&script_dir).unwrap();
        let mut command = Command::new(POWERSHELL_PROGRAM);
        command
            .arg("-NoLogo")
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(&script)
            .arg("-InstallDir")
            .arg(&paths.install)
            .arg("-StagingDir")
            .arg(&paths.stage)
            .arg("-ManifestPath")
            .arg(&paths.manifest)
            .arg("-BackupDir")
            .arg(&paths.backup)
            .arg("-LogPath")
            .arg(&paths.log)
            .arg("-WaitSeconds")
            .arg(wait_seconds.to_string());
        if inject_failure {
            command.arg("-TestFailAfterAliasesReplace");
        }
        command.output().unwrap()
    }

    fn assert_exit(output: &Output, expected: i32) {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(expected),
            "stdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }

    fn assert_all_aliases(install: &Path, expected: &[u8]) {
        for alias in crate::update::install::ALIAS_EXES {
            let bytes = fs::read(install.join(alias)).unwrap();
            assert_eq!(bytes, expected, "{alias} was not updated/restored");
        }
    }

    fn cleanup(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn helper_replaces_all_aliases_and_assets_and_preserves_unrelated_files() {
        let root = temp_root("success");
        let install = make_install(&root, &crate::update::install::ALIAS_EXES);
        let paths = paths(&root, install.clone());

        let output = run_helper(&paths, 15, false);
        assert_exit(&output, 0);

        assert_all_aliases(&install, NEW_BINARY);
        assert!(install.join("assets/akari/idle/new.png").is_file());
        assert!(install.join("assets/akari/sweep/new.png").is_file());
        assert!(
            !install.join("assets/akari/idle/old.png").exists(),
            "old sprites must not remain after replacement"
        );
        assert_eq!(
            fs::read(install.join("settings.json")).unwrap(),
            b"{\"theme\":\"akari\"}"
        );
        assert_eq!(
            fs::read(install.join("user data").join("keep.txt")).unwrap(),
            b"keep-me"
        );
        assert!(!paths.backup.exists(), "backup must be removed on success");
        assert!(!paths.stage.exists(), "staging must be removed on success");
        assert!(!paths.manifest.exists());
        assert!(
            !paths.workspace.exists(),
            "transaction workspace must be removed"
        );
        assert!(paths.log.is_file(), "persistent update log must remain");
        cleanup(&root);
    }

    #[test]
    fn helper_creates_previously_absent_aliases_on_success() {
        let root = temp_root("absent success");
        let install = make_install(&root, &["akari.exe"]);
        let paths = paths(&root, install.clone());

        let output = run_helper(&paths, 15, false);
        assert_exit(&output, 0);
        assert_all_aliases(&install, NEW_BINARY);
        cleanup(&root);
    }

    #[test]
    fn injected_replacement_failure_restores_aliases_assets_and_absence() {
        let root = temp_root("rollback");
        let install = make_install(&root, &["akari.exe", "akariclean.exe"]);
        let paths = paths(&root, install.clone());

        let output = run_helper(&paths, 15, true);
        assert_exit(&output, 1);

        assert_eq!(fs::read(install.join("akari.exe")).unwrap(), OLD_BINARY);
        assert_eq!(
            fs::read(install.join("akariclean.exe")).unwrap(),
            OLD_BINARY
        );
        for absent in [
            "akarihealth.exe",
            "akariall.exe",
            "akariabout.exe",
            "akarihelp.exe",
        ] {
            assert!(
                !install.join(absent).exists(),
                "rollback must restore the original absence of {absent}"
            );
        }
        assert!(install.join("assets/akari/idle/old.png").is_file());
        assert!(!install.join("assets/akari/idle/new.png").exists());
        assert_eq!(
            fs::read(install.join("settings.json")).unwrap(),
            b"{\"theme\":\"akari\"}"
        );
        assert!(!paths.backup.exists(), "successful rollback removes backup");
        let log = fs::read_to_string(&paths.log).unwrap();
        assert!(
            log.contains("Rollback completed"),
            "rollback outcome must be recorded:\n{log}"
        );
        cleanup(&root);
    }

    #[test]
    fn helper_rejects_a_tampered_staging_tree_before_modifying_anything() {
        let root = temp_root("tampered staging");
        let install = make_install(&root, &crate::update::install::ALIAS_EXES);
        let paths = paths(&root, install.clone());
        fs::write(paths.stage.join("akari-sweeps.exe"), b"MZtampered").unwrap();

        let output = run_helper(&paths, 15, false);
        assert_exit(&output, 1);
        assert_all_aliases(&install, OLD_BINARY);
        assert!(install.join("assets/akari/idle/old.png").is_file());
        assert!(!install.join("assets/akari/idle/new.png").exists());
        let log = fs::read_to_string(&paths.log).unwrap();
        assert!(
            log.to_ascii_lowercase().contains("checksum"),
            "manifest mismatch must be explained:\n{log}"
        );
        cleanup(&root);
    }

    #[test]
    fn locked_executable_times_out_without_changing_the_installation() {
        let root = temp_root("locked");
        let install = make_install(&root, &crate::update::install::ALIAS_EXES);
        let paths = paths(&root, install.clone());

        let locked_path = install.join("akari.exe");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&locked_path)
            .unwrap();
        let output = run_helper(&paths, 1, false);
        drop(lock);
        assert_exit(&output, 3);

        assert_all_aliases(&install, OLD_BINARY);
        assert!(install.join("assets/akari/idle/old.png").is_file());
        assert!(!install.join("assets/akari/idle/new.png").exists());
        let log = fs::read_to_string(&paths.log).unwrap();
        assert!(
            log.to_ascii_lowercase().contains("timed out"),
            "timeout must be explained:\n{log}"
        );
        cleanup(&root);
    }
}
