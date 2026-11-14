//! Downloading, SHA-256 verification, and safe ZIP extraction.
//!
//! The archive is untrusted input: it is verified before extraction, every
//! entry name is validated, only the expected executable and `assets/` tree
//! are accepted, symlinks/absolute paths/traversal are rejected, and
//! extraction happens in an isolated staging directory.

use super::github::{self, SelectedAssets};
use super::{Result, UpdateError};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const CANONICAL_EXE: &str = "akari-sweeps.exe";
pub const ASSETS_DIR: &str = "assets";

const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;

/// Maximum compressed release archive size accepted from GitHub. The
/// legitimate Akari-Sweeps package is a few megabytes; 100 MiB leaves ample
/// room for future sprite sets while bounding memory/disk exposure.
pub const MAX_COMPRESSED_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;

/// GitHub currently performs a single redirect to its signed CDN host. A
/// small fixed cap prevents redirect loops and chained-resource abuse.
const MAX_REDIRECTS: usize = 5;

/// Temporary workspace for one update transaction. Callers explicitly
/// remove it on failure. It is intentionally not a `Drop` guard: after a
/// successful launch the PowerShell helper owns the directory and the
/// updater process exits immediately.
#[derive(Debug)]
pub struct UpdateWorkspace {
    pub root: PathBuf,
}

impl UpdateWorkspace {
    pub fn create() -> Result<Self> {
        let base = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for attempt in 0..32u32 {
            let root = base.join(format!("akari-sweeps-update-{pid}-{nanos}-{attempt}"));
            match fs::create_dir(&root) {
                Ok(()) => return Ok(Self { root }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(UpdateError::Io(format!(
                        "could not create a temporary update workspace ({}): {e}",
                        root.display()
                    )));
                }
            }
        }
        Err(UpdateError::Io(
            "could not create a unique temporary update workspace".to_string(),
        ))
    }

    pub fn cleanup(&self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Debug, Clone)]
pub struct VerifiedDownload {
    pub zip_path: PathBuf,
    pub sha256: String,
}

pub fn download_release(
    assets: &SelectedAssets,
    workspace: &UpdateWorkspace,
) -> Result<VerifiedDownload> {
    let zip_path = workspace.root.join(github::WINDOWS_PACKAGE);
    let checksum_path = workspace.root.join(github::CHECKSUM_ASSET);

    download_file(&assets.zip.browser_download_url, &zip_path)?;
    download_file(&assets.checksum.browser_download_url, &checksum_path)?;

    let checksum_text = fs::read_to_string(&checksum_path).map_err(|e| {
        UpdateError::Io(format!(
            "could not read the downloaded checksum file ({}): {e}",
            checksum_path.display()
        ))
    })?;

    match verify_checksum_file(&zip_path, &checksum_text, assets.zip_digest.as_deref()) {
        Ok(hash) => Ok(VerifiedDownload {
            zip_path,
            sha256: hash,
        }),
        Err(e) => {
            // Never leave an unverified archive behind.
            let _ = fs::remove_file(&zip_path);
            let _ = fs::remove_file(&checksum_path);
            Err(e)
        }
    }
}

pub fn download_file(url: &str, destination: &Path) -> Result<()> {
    let asset_name = destination
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("asset");
    github::validate_download_url(url, asset_name)?;

    let client = reqwest::blocking::Client::builder()
        .user_agent(github::USER_AGENT)
        .connect_timeout(DOWNLOAD_CONNECT_TIMEOUT)
        .timeout(DOWNLOAD_TIMEOUT)
        // Redirects are followed manually so every destination can be
        // validated before a request is sent.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| UpdateError::Network(format!("could not create the HTTP client ({e})")))?;

    let response = follow_redirects(&client, url, asset_name, github::validate_redirect_url)?;
    receive_limited(
        response,
        destination,
        MAX_COMPRESSED_DOWNLOAD_BYTES,
        asset_name,
    )
}

/// Follow redirects with an explicit hop limit and a validator called before
/// each destination is requested.
fn follow_redirects<F>(
    client: &reqwest::blocking::Client,
    initial: &str,
    asset_name: &str,
    validate_redirect: F,
) -> Result<reqwest::blocking::Response>
where
    F: Fn(&str, &str) -> Result<()>,
{
    let mut current = reqwest::Url::parse(initial).map_err(|_| {
        UpdateError::InvalidRelease(format!("asset '{asset_name}' has an invalid download URL"))
    })?;

    for hop in 0..=MAX_REDIRECTS {
        let response = client
            .get(current.clone())
            .header(reqwest::header::ACCEPT, "application/octet-stream")
            .send()
            .map_err(|e| UpdateError::Network(format!("could not download {asset_name} ({e})")))?;

        if !response.status().is_redirection() {
            if !response.status().is_success() {
                return Err(UpdateError::Network(format!(
                    "download of {asset_name} returned HTTP {}",
                    response.status().as_u16()
                )));
            }
            return Ok(response);
        }

        if hop == MAX_REDIRECTS {
            return Err(UpdateError::Network(format!(
                "download of {asset_name} exceeded the redirect limit ({MAX_REDIRECTS})"
            )));
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                UpdateError::Network(format!(
                    "download of {asset_name} returned a redirect without a Location header"
                ))
            })?;
        let next = current.join(location).map_err(|_| {
            UpdateError::Network(format!(
                "download of {asset_name} returned an invalid redirect location"
            ))
        })?;
        validate_redirect(next.as_str(), asset_name)?;
        current = next;
    }

    Err(UpdateError::Network(format!(
        "download of {asset_name} exceeded the redirect limit"
    )))
}

/// Write a response body while enforcing a hard byte ceiling. Both the
/// declared `Content-Length` and the actual streamed byte count are checked;
/// neither can bypass the limit. Incomplete files are removed on failure.
fn receive_limited(
    mut response: reqwest::blocking::Response,
    destination: &Path,
    max_bytes: u64,
    asset_name: &str,
) -> Result<()> {
    if let Some(declared) = declared_content_length(&response) {
        if declared > max_bytes {
            return Err(UpdateError::Network(format!(
                "download of {asset_name} declares {declared} bytes, which exceeds the \
                 {max_bytes}-byte limit"
            )));
        }
    }

    let mut file = File::create(destination)
        .map_err(|e| UpdateError::Io(format!("could not create {}: {e}", destination.display())))?;
    let result = (|| {
        let mut limited = (&mut response).take(max_bytes.saturating_add(1));
        let written = io::copy(&mut limited, &mut file)?;
        file.flush()?;
        file.sync_all()?;
        Ok::<u64, io::Error>(written)
    })();

    match result {
        Ok(written) if written > max_bytes => {
            drop(file);
            let _ = fs::remove_file(destination);
            Err(UpdateError::Network(format!(
                "download of {asset_name} exceeded the {max_bytes}-byte limit"
            )))
        }
        Ok(_) => Ok(()),
        Err(e) => {
            drop(file);
            let _ = fs::remove_file(destination);
            Err(UpdateError::Network(format!(
                "download was interrupted while writing {}: {e}",
                destination.display()
            )))
        }
    }
}

fn declared_content_length(response: &reqwest::blocking::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

/// Parse the first non-empty checksum line. Standard `sha256sum` format
/// (`<hex>  <filename>`) is accepted, as is a bare 64-character hash.
pub fn parse_checksum_sidecar(text: &str) -> Result<String> {
    let text = text.trim_start_matches('\u{feff}');
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| UpdateError::Checksum("the checksum file is empty".to_string()))?;
    let hash = line
        .split_once(char::is_whitespace)
        .map(|(hash, _rest)| hash)
        .unwrap_or(line)
        .trim();
    validate_sha256_hex(hash).map(|_| hash.to_ascii_lowercase())
}

/// Parse the optional GitHub Release API digest (`sha256:<hex>`).
pub fn parse_github_digest(digest: &str) -> Result<String> {
    let trimmed = digest.trim();
    let hash = trimmed.strip_prefix("sha256:").ok_or_else(|| {
        UpdateError::Checksum(format!(
            "GitHub supplied an unsupported asset digest '{trimmed}'"
        ))
    })?;
    validate_sha256_hex(hash).map(|_| hash.to_ascii_lowercase())
}

fn validate_sha256_hex(hash: &str) -> Result<()> {
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(UpdateError::Checksum(
            "the checksum is not a valid 64-character SHA-256 hexadecimal value".to_string(),
        ));
    }
    Ok(())
}

/// Verify the sidecar and optional GitHub digest against the exact file.
pub fn verify_checksum_file(
    zip_path: &Path,
    sidecar_text: &str,
    api_digest: Option<&str>,
) -> Result<String> {
    let expected = parse_checksum_sidecar(sidecar_text)?;
    let actual = sha256_file(zip_path)?;
    if expected != actual {
        return Err(UpdateError::Checksum(format!(
            "SHA-256 checksum mismatch: published {expected}, downloaded {actual}"
        )));
    }
    if let Some(digest) = api_digest {
        let api_hash = parse_github_digest(digest)?;
        if api_hash != actual {
            return Err(UpdateError::Checksum(format!(
                "the GitHub API digest does not match the downloaded archive (API {api_hash}, downloaded {actual})"
            )));
        }
    }
    Ok(actual)
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let file = File::open(path)
        .map_err(|e| UpdateError::Io(format!("could not open {}: {e}", path.display())))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    io::copy(&mut reader, &mut hasher)
        .map_err(|e| UpdateError::Io(format!("could not read {}: {e}", path.display())))?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Extract a verified package into a fresh staging directory.
///
/// Returns the number of files extracted. On any validation/extraction
/// failure the staging directory is removed, leaving no partial update.
pub fn extract_package(zip_path: &Path, staging_dir: &Path) -> Result<usize> {
    let result = extract_package_inner(zip_path, staging_dir);
    if result.is_err() {
        let _ = fs::remove_dir_all(staging_dir);
    }
    result
}

fn extract_package_inner(zip_path: &Path, staging_dir: &Path) -> Result<usize> {
    prepare_empty_dir(staging_dir)?;
    let file = File::open(zip_path)
        .map_err(|e| UpdateError::Archive(format!("could not open the ZIP archive: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| UpdateError::Archive(format!("the downloaded ZIP is corrupt ({e})")))?;

    let mut seen: HashSet<String> = HashSet::new();
    let mut extracted_files = 0usize;
    let mut total_bytes = 0u64;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| UpdateError::Archive(format!("could not read ZIP entry {index}: {e}")))?;

        if entry.is_symlink() || has_unix_symlink_mode(entry.unix_mode()) {
            return Err(UpdateError::Archive(format!(
                "ZIP entry '{}' is a symbolic link, which is not allowed",
                entry.name()
            )));
        }
        if entry.enclosed_name().is_none() {
            return Err(UpdateError::Archive(format!(
                "ZIP entry '{}' has an unsafe path",
                entry.name()
            )));
        }

        let parts = validated_components(entry.name(), entry.is_dir())?;
        let (relative, normalized_key, is_canonical) =
            classify_entry(&parts, entry.name(), entry.is_dir())?;
        if !seen.insert(normalized_key) {
            return Err(UpdateError::Archive(format!(
                "ZIP contains a duplicate entry '{}'",
                entry.name()
            )));
        }

        let destination = staging_dir.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&destination).map_err(|e| {
                UpdateError::Archive(format!(
                    "could not create staging directory {}: {e}",
                    destination.display()
                ))
            })?;
            continue;
        }

        if entry.size() > MAX_ENTRY_BYTES {
            return Err(UpdateError::Archive(format!(
                "ZIP entry '{}' is too large ({})",
                entry.name(),
                entry.size()
            )));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                UpdateError::Archive(format!(
                    "could not create staging directory {}: {e}",
                    parent.display()
                ))
            })?;
        }
        let mut output = File::create(&destination).map_err(|e| {
            UpdateError::Archive(format!(
                "could not create staged file {}: {e}",
                destination.display()
            ))
        })?;
        let mut limited = (&mut entry).take(MAX_ENTRY_BYTES + 1);
        let written = io::copy(&mut limited, &mut output).map_err(|e| {
            UpdateError::Archive(format!(
                "could not extract ZIP entry '{}': {e}",
                entry.name()
            ))
        })?;
        output
            .flush()
            .and_then(|_| output.sync_all())
            .map_err(|e| {
                UpdateError::Archive(format!(
                    "could not finish writing {}: {e}",
                    destination.display()
                ))
            })?;
        if written > MAX_ENTRY_BYTES {
            return Err(UpdateError::Archive(format!(
                "ZIP entry '{}' exceeds the extraction size limit",
                entry.name()
            )));
        }
        total_bytes = total_bytes.saturating_add(written);
        if total_bytes > MAX_TOTAL_BYTES {
            return Err(UpdateError::Archive(
                "the ZIP expands beyond the extraction size limit".to_string(),
            ));
        }
        if is_canonical && written < 2 {
            return Err(UpdateError::Archive(
                "the packaged Akari-Sweeps executable is empty".to_string(),
            ));
        }
        extracted_files += 1;
    }

    validate_staging_layout(staging_dir)?;
    Ok(extracted_files)
}

fn prepare_empty_dir(dir: &Path) -> Result<()> {
    if dir.exists() {
        if !dir.is_dir() {
            return Err(UpdateError::Archive(format!(
                "staging path {} is not a directory",
                dir.display()
            )));
        }
        let mut entries = fs::read_dir(dir).map_err(|e| {
            UpdateError::Archive(format!(
                "could not inspect staging directory {}: {e}",
                dir.display()
            ))
        })?;
        if entries.next().is_some() {
            return Err(UpdateError::Archive(format!(
                "staging directory {} is not empty",
                dir.display()
            )));
        }
    } else {
        fs::create_dir_all(dir).map_err(|e| {
            UpdateError::Archive(format!(
                "could not create staging directory {}: {e}",
                dir.display()
            ))
        })?;
    }
    Ok(())
}

/// Validate a ZIP entry name without trusting platform-specific path
/// semantics. Both `/` and `\` are treated as separators, and drive roots,
/// absolute paths, `..`, control characters, alternate data streams, and
/// trailing dot/space names are rejected.
fn validated_components(raw: &str, is_dir: bool) -> Result<Vec<String>> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(invalid_archive_path(raw, "empty or contains NUL"));
    }
    let unified = raw.replace('\\', "/");
    if unified.starts_with('/') {
        return Err(invalid_archive_path(raw, "is absolute"));
    }
    let trimmed = unified.trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(invalid_archive_path(raw, "has no file name"));
    }
    if !is_dir && unified.len() != trimmed.len() {
        return Err(invalid_archive_path(raw, "is a file with a trailing slash"));
    }
    let mut out = Vec::new();
    for component in trimmed.split('/') {
        if component.is_empty() {
            return Err(invalid_archive_path(raw, "contains an empty component"));
        }
        if component == "." || component == ".." {
            return Err(invalid_archive_path(raw, "contains a traversal component"));
        }
        if component.contains(':') {
            return Err(invalid_archive_path(
                raw,
                "contains a drive or alternate-data-stream separator",
            ));
        }
        if component.ends_with(' ') || component.ends_with('.') {
            return Err(invalid_archive_path(
                raw,
                "contains a name Windows would normalize",
            ));
        }
        if component.chars().any(|c| c.is_control()) {
            return Err(invalid_archive_path(raw, "contains control characters"));
        }
        out.push(component.to_string());
    }
    if out.is_empty() {
        return Err(invalid_archive_path(raw, "has no file name"));
    }
    Ok(out)
}

fn invalid_archive_path(raw: &str, why: &str) -> UpdateError {
    UpdateError::Archive(format!("ZIP entry '{raw}' {why}"))
}

/// Some writers do not set the ZIP "version made by" host bits that
/// `is_symlink()` relies on, so also inspect the Unix mode directly. Any
/// reparse-like entry is rejected rather than extracted as a regular file.
fn has_unix_symlink_mode(mode: Option<u32>) -> bool {
    mode.is_some_and(|mode| mode & 0o170000 == 0o120000)
}

fn classify_entry(parts: &[String], raw: &str, is_dir: bool) -> Result<(PathBuf, String, bool)> {
    let lower: Vec<String> = parts.iter().map(|p| p.to_ascii_lowercase()).collect();
    let lower_joined = lower.join("/");

    if is_dir {
        if lower_joined != ASSETS_DIR && !lower_joined.starts_with("assets/") {
            return Err(UpdateError::Archive(format!(
                "ZIP entry '{raw}' is not part of the application package"
            )));
        }
        return Ok((PathBuf::from(ASSETS_DIR), lower_joined, false));
    }

    if lower_joined == CANONICAL_EXE {
        return Ok((PathBuf::from(CANONICAL_EXE), lower_joined, true));
    }
    if lower_joined == ASSETS_DIR {
        return Err(UpdateError::Archive(format!(
            "ZIP entry '{raw}' must be a directory"
        )));
    }
    if !lower_joined.starts_with("assets/") {
        return Err(UpdateError::Archive(format!(
            "ZIP entry '{raw}' is not part of the application package"
        )));
    }
    let file_name = lower.last().cloned().unwrap_or_default();
    if is_executable_name(&file_name) {
        return Err(UpdateError::Archive(format!(
            "ZIP entry '{raw}' is an unexpected executable inside assets"
        )));
    }

    let mut relative = PathBuf::from(ASSETS_DIR);
    for part in &parts[1..] {
        relative.push(part);
    }
    Ok((relative, lower_joined, false))
}

fn is_executable_name(file_name: &str) -> bool {
    const EXECUTABLE_EXTENSIONS: &[&str] = &[
        ".exe", ".dll", ".bat", ".cmd", ".com", ".scr", ".ps1", ".msi", ".vbs", ".vbe", ".js",
        ".jse", ".wsf", ".wsh", ".cpl", ".pif", ".hta", ".lnk",
    ];
    EXECUTABLE_EXTENSIONS
        .iter()
        .any(|ext| file_name.ends_with(ext))
}

fn validate_staging_layout(staging_dir: &Path) -> Result<()> {
    let canonical = staging_dir.join(CANONICAL_EXE);
    if !canonical.is_file() {
        return Err(UpdateError::Archive(format!(
            "the package is missing '{CANONICAL_EXE}'"
        )));
    }
    let mut header = [0u8; 2];
    let mut file = File::open(&canonical).map_err(|e| {
        UpdateError::Archive(format!(
            "could not open the staged executable {}: {e}",
            canonical.display()
        ))
    })?;
    file.read_exact(&mut header).map_err(|_| {
        UpdateError::Archive("the packaged Akari-Sweeps executable is too small".to_string())
    })?;
    if header != *b"MZ" {
        return Err(UpdateError::Archive(
            "the packaged Akari-Sweeps executable is not a Windows executable".to_string(),
        ));
    }

    let assets = staging_dir.join(ASSETS_DIR);
    if !assets.is_dir() {
        return Err(UpdateError::Archive(
            "the package is missing the assets directory".to_string(),
        ));
    }
    if !has_regular_file(&assets)? {
        return Err(UpdateError::Archive(
            "the packaged assets directory is empty".to_string(),
        ));
    }
    Ok(())
}

fn has_regular_file(dir: &Path) -> Result<bool> {
    let entries = fs::read_dir(dir)
        .map_err(|e| UpdateError::Archive(format!("could not inspect {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            UpdateError::Archive(format!("could not inspect {}: {e}", dir.display()))
        })?;
        let file_type = entry.file_type().map_err(|e| {
            UpdateError::Archive(format!("could not inspect {}: {e}", entry.path().display()))
        })?;
        if file_type.is_file() {
            return Ok(true);
        }
        if file_type.is_dir() && has_regular_file(&entry.path())? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Write a deterministic `<sha256>  <relative/path>` manifest for every
/// staged file. The PowerShell helper verifies this manifest before and
/// after installation. UTF-8 without BOM.
pub fn write_manifest(staging_dir: &Path, manifest_path: &Path) -> Result<usize> {
    let mut files = Vec::new();
    collect_files(staging_dir, staging_dir, &mut files)?;
    files.sort();
    let mut content = String::new();
    for relative in &files {
        let absolute = staging_dir.join(relative);
        let hash = sha256_file(&absolute)?;
        let portable = relative
            .components()
            .filter_map(|component| match component {
                Component::Normal(part) => Some(part.to_string_lossy().to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        content.push_str(&format!("{hash}  {portable}\n"));
    }
    fs::write(manifest_path, content.as_bytes()).map_err(|e| {
        UpdateError::Io(format!(
            "could not write the staging manifest {}: {e}",
            manifest_path.display()
        ))
    })?;
    Ok(files.len())
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(dir)
        .map_err(|e| UpdateError::Io(format!("could not inspect {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry
            .map_err(|e| UpdateError::Io(format!("could not inspect {}: {e}", dir.display())))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| UpdateError::Io(format!("could not inspect {}: {e}", path.display())))?;
        if file_type.is_symlink() {
            return Err(UpdateError::Archive(format!(
                "staged path {} is a symbolic link",
                path.display()
            )));
        }
        if file_type.is_dir() {
            collect_files(root, &path, out)?;
        } else if file_type.is_file() {
            let relative = path.strip_prefix(root).map_err(|_| {
                UpdateError::Io(format!(
                    "staged file {} escaped the staging directory",
                    path.display()
                ))
            })?;
            out.push(relative.to_path_buf());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use zip::write::SimpleFileOptions;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "akari-update-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8], bool)]) {
        let file = File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        for (name, data, is_dir) in entries {
            if *is_dir {
                writer.add_directory(*name, options).unwrap();
            } else {
                writer.start_file(*name, options).unwrap();
                writer.write_all(data).unwrap();
            }
        }
        writer.finish().unwrap();
    }

    fn valid_entries() -> Vec<(&'static str, &'static [u8], bool)> {
        vec![
            ("akari-sweeps.exe", b"MZfake-executable", false),
            ("assets/", b"", true),
            ("assets/akari/sweep/akari_sweep_001.png", b"png-data", false),
            ("assets/akari/idle/README.md", b"notes", false),
        ]
    }

    fn checksum_text(hash: &str) -> String {
        format!("{hash}  {}", github::WINDOWS_PACKAGE)
    }

    fn sample_hash(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }

    #[test]
    fn correct_checksum_passes() {
        let dir = temp_dir("checksum-ok");
        let zip = dir.join("package.zip");
        fs::write(&zip, b"archive").unwrap();
        let hash = sample_hash(b"archive");
        assert_eq!(
            verify_checksum_file(&zip, &checksum_text(&hash), None).unwrap(),
            hash
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn incorrect_checksum_fails() {
        let dir = temp_dir("checksum-bad");
        let zip = dir.join("package.zip");
        fs::write(&zip, b"archive").unwrap();
        let wrong = "0".repeat(64);
        let err = verify_checksum_file(&zip, &checksum_text(&wrong), None).unwrap_err();
        assert!(matches!(err, UpdateError::Checksum(_)));
        assert!(err.to_string().contains("mismatch"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_checksum_text_fails() {
        for text in [
            "",
            "not-a-hash",
            &"a".repeat(63),
            &"g".repeat(64),
            "\u{feff}",
        ] {
            assert!(
                parse_checksum_sidecar(text).is_err(),
                "{text:?} must be rejected"
            );
        }
    }

    #[test]
    fn standard_sidecar_formats_are_accepted() {
        let hash = "a".repeat(64);
        assert_eq!(
            parse_checksum_sidecar(&format!("{hash}  package.zip")).unwrap(),
            hash
        );
        assert_eq!(
            parse_checksum_sidecar(&format!("{hash} *package.zip")).unwrap(),
            hash
        );
        assert_eq!(parse_checksum_sidecar(&hash).unwrap(), hash);
        assert_eq!(
            parse_checksum_sidecar(&format!("\u{feff}{hash}  package.zip\r\n")).unwrap(),
            hash
        );
    }

    #[test]
    fn malformed_github_digest_fails() {
        for digest in ["", "md5:abc", "sha256:", "sha256:xyz", "sha256:1234"] {
            assert!(
                parse_github_digest(digest).is_err(),
                "{digest:?} must be rejected"
            );
        }
    }

    #[test]
    fn api_digest_is_cross_checked() {
        let dir = temp_dir("digest-cross-check");
        let zip = dir.join("package.zip");
        fs::write(&zip, b"archive").unwrap();
        let hash = sample_hash(b"archive");
        let ok = format!("sha256:{hash}");
        assert!(verify_checksum_file(&zip, &checksum_text(&hash), Some(&ok)).is_ok());

        let wrong_api = format!("sha256:{}", "0".repeat(64));
        let err = verify_checksum_file(&zip, &checksum_text(&hash), Some(&wrong_api)).unwrap_err();
        assert!(err.to_string().contains("GitHub API digest"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn valid_package_extracts_successfully() {
        let dir = temp_dir("extract-ok");
        let zip = dir.join("package.zip");
        let staging = dir.join("staging");
        write_zip(&zip, &valid_entries());
        assert_eq!(extract_package(&zip, &staging).unwrap(), 3);
        assert!(staging.join(CANONICAL_EXE).is_file());
        assert!(
            staging
                .join("assets/akari/sweep/akari_sweep_001.png")
                .is_file()
        );
        assert!(staging.join("assets/akari/idle/README.md").is_file());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn backslash_separated_package_extracts_successfully() {
        // Windows' Compress-Archive historically writes backslash separators.
        // The release workflow uses it, so the updater must accept that form.
        let dir = temp_dir("extract-backslash");
        let zip = dir.join("package.zip");
        let staging = dir.join("staging");
        write_zip(
            &zip,
            &[
                ("akari-sweeps.exe", b"MZfake-executable", false),
                ("assets\\", b"", true),
                (
                    "assets\\akari\\sweep\\akari_sweep_001.png",
                    b"png-data",
                    false,
                ),
            ],
        );
        assert_eq!(extract_package(&zip, &staging).unwrap(), 2);
        assert!(staging.join(CANONICAL_EXE).is_file());
        assert!(
            staging
                .join("assets/akari/sweep/akari_sweep_001.png")
                .is_file()
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_executable_is_rejected() {
        let dir = temp_dir("extract-no-exe");
        let zip = dir.join("package.zip");
        let staging = dir.join("staging");
        write_zip(
            &zip,
            &[
                ("assets/", b"", true),
                ("assets/akari/sweep/akari_sweep_001.png", b"png", false),
            ],
        );
        let err = extract_package(&zip, &staging).unwrap_err();
        assert!(err.to_string().contains("missing 'akari-sweeps.exe'"));
        assert!(!staging.exists(), "failed staging data must be removed");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_assets_directory_is_rejected() {
        let dir = temp_dir("extract-no-assets");
        let zip = dir.join("package.zip");
        let staging = dir.join("staging");
        write_zip(&zip, &[("akari-sweeps.exe", b"MZfake", false)]);
        let err = extract_package(&zip, &staging).unwrap_err();
        assert!(err.to_string().contains("assets"));
        assert!(!staging.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_zip_is_rejected() {
        let dir = temp_dir("extract-corrupt");
        let zip = dir.join("package.zip");
        let staging = dir.join("staging");
        fs::write(&zip, b"this is not a zip file").unwrap();
        let err = extract_package(&zip, &staging).unwrap_err();
        assert!(matches!(err, UpdateError::Archive(_)));
        assert!(!staging.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn directory_traversal_is_rejected() {
        for name in [
            "../evil.exe",
            "assets/../../evil.exe",
            "assets\\..\\..\\evil.exe",
            "./evil.exe",
        ] {
            let dir = temp_dir("extract-traversal");
            let zip = dir.join("package.zip");
            let staging = dir.join("staging");
            write_zip(&zip, &[(name, b"MZevil", false)]);
            let err = extract_package(&zip, &staging).unwrap_err();
            assert!(matches!(err, UpdateError::Archive(_)), "{name}: {err}");
            assert!(!staging.exists());
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn absolute_archive_paths_are_rejected() {
        for name in ["/evil.exe", "\\evil.exe", "C:/evil.exe", "C:\\evil.exe"] {
            let dir = temp_dir("extract-absolute");
            let zip = dir.join("package.zip");
            let staging = dir.join("staging");
            write_zip(&zip, &[(name, b"MZevil", false)]);
            let err = extract_package(&zip, &staging).unwrap_err();
            assert!(matches!(err, UpdateError::Archive(_)), "{name}: {err}");
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn unexpected_root_files_are_rejected() {
        let dir = temp_dir("extract-unexpected");
        let zip = dir.join("package.zip");
        let staging = dir.join("staging");
        let mut entries = valid_entries();
        entries.push(("setup.exe", b"MZ", false));
        write_zip(&zip, &entries);
        let err = extract_package(&zip, &staging).unwrap_err();
        assert!(
            err.to_string()
                .contains("not part of the application package")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn executables_inside_assets_are_rejected() {
        for name in [
            "assets/evil.exe",
            "assets/tool.PS1",
            "assets/akari/helper.dll",
        ] {
            let dir = temp_dir("extract-asset-exe");
            let zip = dir.join("package.zip");
            let staging = dir.join("staging");
            let mut entries = valid_entries();
            entries.push((name, b"MZ", false));
            write_zip(&zip, &entries);
            let err = extract_package(&zip, &staging).unwrap_err();
            assert!(err.to_string().contains("unexpected executable"), "{name}");
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn symlink_entries_are_rejected() {
        let dir = temp_dir("extract-symlink");
        let zip = dir.join("package.zip");
        let staging = dir.join("staging");
        let file = File::create(&zip).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .add_symlink("assets/link", "../outside", SimpleFileOptions::default())
            .unwrap();
        writer.finish().unwrap();
        let err = extract_package(&zip, &staging).unwrap_err();
        assert!(err.to_string().contains("symbolic link"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn duplicate_entries_are_rejected() {
        let dir = temp_dir("extract-duplicate");
        let zip = dir.join("package.zip");
        let staging = dir.join("staging");
        let mut entries = valid_entries();
        entries.push(("ASSETS/akari/IDLE/readme.md", b"duplicate", false));
        write_zip(&zip, &entries);
        let err = extract_package(&zip, &staging).unwrap_err();
        assert!(err.to_string().contains("duplicate"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn manifest_lists_every_staged_file_with_hashes() {
        let dir = temp_dir("manifest");
        let zip = dir.join("package.zip");
        let staging = dir.join("staging");
        let manifest = dir.join("manifest.sha256");
        write_zip(&zip, &valid_entries());
        extract_package(&zip, &staging).unwrap();
        assert_eq!(write_manifest(&staging, &manifest).unwrap(), 3);
        let text = fs::read_to_string(&manifest).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        for line in &lines {
            assert_eq!(&line[64..66], "  ");
        }
        assert!(
            lines
                .iter()
                .any(|line| line.ends_with("  akari-sweeps.exe"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.ends_with("  assets/akari/sweep/akari_sweep_001.png"))
        );
        assert!(text.ends_with('\n'));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn workspace_names_are_unique_and_cleanable() {
        let a = UpdateWorkspace::create().unwrap();
        let b = UpdateWorkspace::create().unwrap();
        assert_ne!(a.root, b.root);
        assert!(a.root.is_dir());
        a.cleanup();
        assert!(!a.root.exists());
        b.cleanup();
    }

    // Small local HTTP server helpers. They make download-limit and redirect
    // tests deterministic without touching GitHub or the user's network.
    use std::net::TcpListener;
    use std::thread;

    fn http_response(status: &str, headers: &[String], body: &[u8]) -> Vec<u8> {
        let mut response = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
        for header in headers {
            response.push_str(header);
            response.push_str("\r\n");
        }
        response.push_str("\r\n");
        let mut bytes = response.into_bytes();
        bytes.extend_from_slice(body);
        bytes
    }

    fn chunked_body(data: &[u8]) -> Vec<u8> {
        let mut body = format!("{:x}\r\n", data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n0\r\n\r\n");
        body
    }

    fn serve_once(raw: Vec<u8>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            stream.write_all(&raw).unwrap();
            stream.flush().unwrap();
        });
        (format!("http://{address}/asset"), handle)
    }

    fn test_client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }

    #[test]
    fn declared_content_length_over_limit_is_rejected_before_writing() {
        let body = vec![b'x'; 100];
        let raw = http_response("200 OK", &["Content-Length: 100".to_string()], &body);
        let (url, server) = serve_once(raw);
        let response = test_client().get(&url).send().unwrap();
        let dest = temp_dir("declared-over").join("asset.zip");
        let err = receive_limited(response, &dest, 10, "asset.zip").unwrap_err();
        assert!(err.to_string().contains("exceeds"), "{err}");
        assert!(!dest.exists(), "rejected download must not leave a file");
        server.join().unwrap();
    }

    #[test]
    fn missing_content_length_under_limit_succeeds() {
        let body = b"0123456789";
        let raw = http_response("200 OK", &[], body);
        let (url, server) = serve_once(raw);
        let response = test_client().get(&url).send().unwrap();
        let dest = temp_dir("missing-length-ok").join("asset.zip");
        receive_limited(response, &dest, 10, "asset.zip").unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);
        server.join().unwrap();
    }

    #[test]
    fn missing_content_length_over_limit_is_stopped_while_streaming() {
        let body = vec![b'x'; 100];
        let raw = http_response(
            "200 OK",
            &["Transfer-Encoding: chunked".to_string()],
            &chunked_body(&body),
        );
        let (url, server) = serve_once(raw);
        let response = test_client().get(&url).send().unwrap();
        let dest = temp_dir("missing-length-over").join("asset.zip");
        let err = receive_limited(response, &dest, 10, "asset.zip").unwrap_err();
        assert!(err.to_string().contains("exceed"), "{err}");
        assert!(!dest.exists(), "incomplete download must be removed");
        server.join().unwrap();
    }

    #[test]
    fn inaccurate_content_length_cannot_bypass_the_stream_limit() {
        // A misleading small Content-Length is ignored in favor of the
        // actual streamed bytes when Transfer-Encoding is chunked.
        let body = vec![b'x'; 100];
        let raw = http_response(
            "200 OK",
            &[
                "Content-Length: 10".to_string(),
                "Transfer-Encoding: chunked".to_string(),
            ],
            &chunked_body(&body),
        );
        let (url, server) = serve_once(raw);
        let response = test_client().get(&url).send().unwrap();
        let dest = temp_dir("inaccurate-length").join("asset.zip");
        let err = receive_limited(response, &dest, 10, "asset.zip").unwrap_err();
        assert!(err.to_string().contains("exceed"), "{err}");
        assert!(!dest.exists());
        server.join().unwrap();
    }

    #[test]
    fn excessive_redirect_chain_is_rejected() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let redirect = http_response(
            "302 Found",
            &[
                "Location: /next".to_string(),
                "Content-Length: 0".to_string(),
            ],
            b"",
        );
        let server = thread::spawn(move || {
            for _ in 0..=MAX_REDIRECTS {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(&redirect);
                let _ = stream.flush();
            }
        });
        let url = format!("http://{address}/start");
        let err = follow_redirects(&test_client(), &url, "asset.zip", |_, _| Ok(())).unwrap_err();
        assert!(err.to_string().contains("redirect limit"), "{err}");
        server.join().unwrap();
    }

    #[test]
    fn rejected_asset_url_never_reaches_extraction_or_installation() {
        let workspace = UpdateWorkspace::create().unwrap();
        let assets = SelectedAssets {
            zip: github::ReleaseAsset {
                name: github::WINDOWS_PACKAGE.to_string(),
                browser_download_url: format!(
                    "https://github.com/other-owner/Akari-Sweeps/releases/download/v9.9.9/{}",
                    github::WINDOWS_PACKAGE
                ),
                digest: None,
            },
            checksum: github::ReleaseAsset {
                name: github::CHECKSUM_ASSET.to_string(),
                browser_download_url: format!(
                    "https://github.com/{}/releases/download/v9.9.9/{}",
                    github::REPOSITORY,
                    github::CHECKSUM_ASSET
                ),
                digest: None,
            },
            zip_digest: None,
        };
        let err = download_release(&assets, &workspace).unwrap_err();
        assert!(matches!(err, UpdateError::InvalidRelease(_)), "{err}");
        let entries = fs::read_dir(&workspace.root).unwrap().count();
        assert_eq!(entries, 0, "no download or staging file may be created");
        assert!(!workspace.root.join("stage").exists());
        workspace.cleanup();
    }
}
