//! Installation-layout discovery and preparation of the Windows helper.
//!
//! The installation directory is always derived from the running executable;
//! `%LOCALAPPDATA%\Akari-Sweeps` is never assumed. This keeps the updater
//! working with custom `install.ps1 -InstallDir` layouts and prevents it from
//! touching a different Akari-Sweeps installation on the same machine.

use super::download::{self, UpdateWorkspace};
use super::powershell::{self, HelperLaunch};
use super::{Result, UpdateError};
use std::fs;
use std::path::{Path, PathBuf};

/// All installed executable aliases. The release archive contains one
/// canonical executable; the helper copies it to every name in this list.
pub const ALIAS_EXES: [&str; 6] = [
    "akari.exe",
    "akariclean.exe",
    "akarihealth.exe",
    "akariall.exe",
    "akariabout.exe",
    "akarihelp.exe",
];

/// The canonical executable produced by Cargo and shipped in the release ZIP.
pub const CANONICAL_EXE: &str = download::CANONICAL_EXE;

#[derive(Debug, Clone)]
pub struct InstallLayout {
    pub directory: PathBuf,
    pub running_exe: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutReport {
    /// Aliases that are absent before the update. The helper creates them
    /// from the canonical executable; rollback removes them again if the
    /// transaction fails.
    pub missing_aliases: Vec<&'static str>,
}

impl InstallLayout {
    pub fn discover() -> Result<Self> {
        let exe = std::env::current_exe().map_err(|e| {
            UpdateError::UnsupportedInstallation(format!(
                "the running executable path could not be determined ({e})"
            ))
        })?;
        Self::from_exe(&exe)
    }

    /// Derive the installation directory from the executable that is
    /// actually running, then validate its name. Only the six aliases and
    /// the canonical Cargo binary are accepted so a stray executable in an
    /// unrelated directory cannot be mistaken for an installation.
    pub fn from_exe(exe: &Path) -> Result<Self> {
        let name = exe.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
            UpdateError::UnsupportedInstallation(
                "the running executable has no file name".to_string(),
            )
        })?;
        let is_alias = ALIAS_EXES
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(name));
        let is_canonical = CANONICAL_EXE.eq_ignore_ascii_case(name);
        if !is_alias && !is_canonical {
            return Err(UpdateError::UnsupportedInstallation(format!(
                "'{name}' is not an installed Akari-Sweeps command. Expected one of: {} \
                 or {CANONICAL_EXE}.",
                ALIAS_EXES.join(", ")
            )));
        }
        let parent = exe
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| {
                UpdateError::UnsupportedInstallation(
                    "the running executable is not inside an installation directory".to_string(),
                )
            })?;
        Ok(Self {
            directory: parent.to_path_buf(),
            running_exe: exe.to_path_buf(),
        })
    }

    /// Check that this is a complete-enough Akari-Sweeps installation to
    /// update safely. Missing non-running aliases are reported (the helper
    /// repairs them), while a missing assets tree or a directory with no
    /// aliases at all is rejected instead of guessing another location.
    pub fn validate(&self) -> Result<LayoutReport> {
        if !self.directory.is_dir() {
            return Err(UpdateError::UnsupportedInstallation(format!(
                "installation directory '{}' does not exist or is not a directory",
                self.directory.display()
            )));
        }
        if !self.running_exe.is_file() {
            return Err(UpdateError::UnsupportedInstallation(format!(
                "the running executable '{}' could not be found",
                self.running_exe.display()
            )));
        }

        let assets = self.directory.join(download::ASSETS_DIR);
        if !assets.is_dir() {
            return Err(UpdateError::UnsupportedInstallation(format!(
                "'{}' is missing. This does not look like a complete Akari-Sweeps \
                 installation; reinstall it with install.ps1 before using the updater.",
                assets.display()
            )));
        }
        let sprite_root = assets.join("akari");
        if !sprite_root.is_dir() {
            return Err(UpdateError::UnsupportedInstallation(format!(
                "the sprite asset directory '{}' is missing. Reinstall Akari-Sweeps \
                 with install.ps1 before updating.",
                sprite_root.display()
            )));
        }
        if !contains_png(&sprite_root)? {
            return Err(UpdateError::UnsupportedInstallation(format!(
                "no sprite PNG files were found under '{}'. Reinstall Akari-Sweeps \
                 with install.ps1 before updating.",
                sprite_root.display()
            )));
        }

        let mut missing_aliases = Vec::new();
        let mut present_aliases = 0usize;
        for alias in ALIAS_EXES {
            if self.directory.join(alias).is_file() {
                present_aliases += 1;
            } else {
                missing_aliases.push(alias);
            }
        }
        if present_aliases == 0 {
            return Err(UpdateError::UnsupportedInstallation(format!(
                "no executable aliases were found in '{}'. Run Akari-Sweeps from its \
                 installed directory or reinstall it with install.ps1.",
                self.directory.display()
            )));
        }

        Ok(LayoutReport { missing_aliases })
    }
}

/// Find at least one PNG below the sprite directory without following
/// reparse points into unrelated locations.
fn contains_png(dir: &Path) -> Result<bool> {
    let entries = fs::read_dir(dir).map_err(|e| {
        UpdateError::UnsupportedInstallation(format!(
            "could not inspect sprite assets at '{}': {e}",
            dir.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            UpdateError::UnsupportedInstallation(format!(
                "could not inspect sprite assets at '{}': {e}",
                dir.display()
            ))
        })?;
        let file_type = entry.file_type().map_err(|e| {
            UpdateError::UnsupportedInstallation(format!(
                "could not inspect '{}': {e}",
                entry.path().display()
            ))
        })?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if contains_png(&entry.path())? {
                return Ok(true);
            }
        } else if file_type.is_file()
            && entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Extract a verified package, generate the Rust-owned SHA-256 manifest, and
/// write the embedded PowerShell helper into the transaction workspace.
/// Nothing in the installation directory is modified by this function.
pub fn prepare_installation(
    workspace: &UpdateWorkspace,
    layout: &InstallLayout,
    zip_path: &Path,
) -> Result<HelperLaunch> {
    ensure_disjoint_paths(&layout.directory, &workspace.root)?;

    let staging_dir = workspace.root.join("stage");
    download::extract_package(zip_path, &staging_dir)?;

    let manifest_path = workspace.root.join("install-manifest.sha256");
    download::write_manifest(&staging_dir, &manifest_path)?;

    let script_path = powershell::write_script(&workspace.root)?;
    let backup_dir = workspace.root.join("backup");
    let log_path = powershell::log_path();

    Ok(HelperLaunch {
        script_path,
        install_dir: layout.directory.clone(),
        staging_dir,
        manifest_path,
        backup_dir,
        log_path,
        wait_seconds: powershell::DEFAULT_WAIT_SECONDS,
    })
}

/// The helper must never recursively delete its own workspace or the
/// installation. This is checked after canonicalization so `..`, junctions,
/// and relative paths cannot make the two locations overlap.
pub fn ensure_disjoint_paths(install_dir: &Path, workspace_root: &Path) -> Result<()> {
    let install = canonical_or_absolute(install_dir)?;
    let workspace = canonical_or_absolute(workspace_root)?;
    if install.starts_with(&workspace) || workspace.starts_with(&install) {
        return Err(UpdateError::UnsupportedInstallation(format!(
            "the update workspace '{}' overlaps the installation directory '{}'; \
             refusing to modify either location",
            workspace.display(),
            install.display()
        )));
    }
    Ok(())
}

fn canonical_or_absolute(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| UpdateError::Io(format!("could not read the current directory: {e}")))?
            .join(path)
    };
    Ok(fs::canonicalize(&absolute).unwrap_or(absolute))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::time::{SystemTime, UNIX_EPOCH};
    use zip::write::SimpleFileOptions;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "akari install {label} {}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_install(label: &str) -> PathBuf {
        let dir = temp_dir(label);
        let sprites = dir.join("assets").join("akari").join("idle");
        fs::create_dir_all(&sprites).unwrap();
        fs::write(sprites.join("idle_001.png"), b"png").unwrap();
        for alias in ALIAS_EXES {
            fs::write(dir.join(alias), b"MZold-binary").unwrap();
        }
        dir
    }

    fn layout(install: &Path, running: &str) -> InstallLayout {
        InstallLayout::from_exe(&install.join(running)).unwrap()
    }

    #[test]
    fn alias_list_is_exactly_the_six_installed_names() {
        assert_eq!(ALIAS_EXES.len(), 6);
        assert_eq!(
            ALIAS_EXES,
            [
                "akari.exe",
                "akariclean.exe",
                "akarihealth.exe",
                "akariall.exe",
                "akariabout.exe",
                "akarihelp.exe"
            ]
        );
        assert!(!ALIAS_EXES.contains(&CANONICAL_EXE));
    }

    #[test]
    fn all_six_aliases_and_custom_directory_are_accepted() {
        let install = make_install("custom path with spaces");
        let report = layout(&install, "akari.exe").validate().unwrap();
        assert!(report.missing_aliases.is_empty());
        assert_eq!(
            layout(&install, "akari.exe").directory,
            install,
            "the install directory must come from the running executable"
        );
        let _ = fs::remove_dir_all(install);
    }

    #[test]
    fn unsupported_running_executable_is_rejected() {
        let install = make_install("unsupported");
        let err = InstallLayout::from_exe(&install.join("random.exe")).unwrap_err();
        assert!(
            err.to_string()
                .contains("not an installed Akari-Sweeps command")
        );
        assert!(InstallLayout::from_exe(&install.join("akari-sweeps.exe")).is_ok());
        let _ = fs::remove_dir_all(install);
    }

    #[test]
    fn missing_assets_directory_is_rejected() {
        let install = make_install("missing assets");
        fs::remove_dir_all(install.join("assets")).unwrap();
        let err = layout(&install, "akari.exe").validate().unwrap_err();
        assert!(err.to_string().contains("missing"));
        let _ = fs::remove_dir_all(install);
    }

    #[test]
    fn assets_without_sprites_are_rejected() {
        let install = make_install("empty sprites");
        fs::remove_dir_all(install.join("assets").join("akari")).unwrap();
        fs::create_dir_all(install.join("assets").join("akari")).unwrap();
        let err = layout(&install, "akari.exe").validate().unwrap_err();
        assert!(err.to_string().contains("no sprite PNG files"));
        let _ = fs::remove_dir_all(install);
    }

    #[test]
    fn missing_aliases_are_reported_and_repaired_by_the_helper() {
        let install = make_install("missing alias");
        fs::remove_file(install.join("akarihelp.exe")).unwrap();
        let report = layout(&install, "akari.exe").validate().unwrap();
        assert_eq!(report.missing_aliases, vec!["akarihelp.exe"]);
        let _ = fs::remove_dir_all(install);
    }

    #[test]
    fn installation_without_any_alias_is_rejected_even_for_canonical_exe() {
        let install = make_install("no aliases");
        for alias in ALIAS_EXES {
            fs::remove_file(install.join(alias)).unwrap();
        }
        fs::write(install.join(CANONICAL_EXE), b"MZcanonical").unwrap();
        let err = layout(&install, CANONICAL_EXE).validate().unwrap_err();
        assert!(err.to_string().contains("no executable aliases"));
        let _ = fs::remove_dir_all(install);
    }

    #[test]
    fn overlapping_workspace_and_installation_are_rejected() {
        let install = temp_dir("overlap install");
        let inside = install.join("update-work");
        fs::create_dir_all(&inside).unwrap();
        assert!(ensure_disjoint_paths(&install, &inside).is_err());
        assert!(ensure_disjoint_paths(&inside, &install).is_err());

        let sibling = temp_dir("overlap sibling");
        assert!(ensure_disjoint_paths(&install, &sibling).is_ok());
        let _ = fs::remove_dir_all(install);
        let _ = fs::remove_dir_all(sibling);
    }

    #[test]
    fn preparation_stages_manifest_and_embedded_helper_without_touching_install() {
        let install = make_install("prepare");
        let layout = layout(&install, "akari.exe");
        let before: Vec<Vec<u8>> = ALIAS_EXES
            .iter()
            .map(|a| fs::read(install.join(a)).unwrap())
            .collect();

        let workspace = UpdateWorkspace::create().unwrap();
        let zip_path = workspace.root.join("package.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        writer.start_file(CANONICAL_EXE, options).unwrap();
        writer.write_all(b"MZnew-binary").unwrap();
        writer.add_directory("assets/", options).unwrap();
        writer
            .start_file("assets/akari/idle/idle_001.png", options)
            .unwrap();
        writer.write_all(b"new-png").unwrap();
        writer.finish().unwrap();

        let launch = prepare_installation(&workspace, &layout, &zip_path).unwrap();
        assert!(launch.staging_dir.join(CANONICAL_EXE).is_file());
        assert!(
            launch
                .staging_dir
                .join("assets/akari/idle/idle_001.png")
                .is_file()
        );
        assert!(launch.manifest_path.is_file());
        assert!(launch.script_path.is_file());
        assert_eq!(launch.install_dir, install);
        assert_eq!(
            launch.log_path.file_name().and_then(|n| n.to_str()),
            Some("update.log")
        );
        assert!(
            launch
                .log_path
                .to_string_lossy()
                .to_ascii_lowercase()
                .contains("akari-sweeps")
        );

        let after: Vec<Vec<u8>> = ALIAS_EXES
            .iter()
            .map(|a| fs::read(install.join(a)).unwrap())
            .collect();
        assert_eq!(before, after, "preparation must not modify the install");

        workspace.cleanup();
        let _ = fs::remove_dir_all(install);
    }
}
