//! GitHub Releases API client and release-asset selection.
//!
//! Only the public `releases/latest` endpoint is used; no authentication is
//! required. Every request carries a User-Agent as required by GitHub, and
//! network/API failures are converted into user-readable errors rather than
//! panics.

use super::{Result, UpdateError};
use crate::update::version;
use serde::Deserialize;
use std::time::Duration;

pub const REPOSITORY: &str = "vishwassharma38/Akari-Sweeps";
pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/vishwassharma38/Akari-Sweeps/releases/latest";
pub const USER_AGENT: &str = "Akari-Sweeps";

/// Exact release asset names produced by `.github/workflows/release.yml`.
pub const WINDOWS_PACKAGE: &str = "akari-sweeps-windows-x86_64.zip";
pub const CHECKSUM_ASSET: &str = "akari-sweeps-windows-x86_64.zip.sha256";

const API_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub html_url: String,
    pub body: Option<String>,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub digest: Option<String>,
}

/// The exact assets the updater will use. Unrelated GitHub assets are
/// ignored by name; the first asset returned by the API is never picked.
#[derive(Debug, Clone)]
pub struct SelectedAssets {
    pub zip: ReleaseAsset,
    pub checksum: ReleaseAsset,
    /// Optional GitHub-provided `sha256:<hex>` digest for the ZIP.
    pub zip_digest: Option<String>,
}

impl Release {
    /// Parse the release tag as a semantic version.
    pub fn version(&self) -> Result<semver::Version> {
        version::parse_release_tag(&self.tag_name)
    }

    /// Select the exact Windows package and checksum sidecar.
    pub fn windows_assets(&self) -> Result<SelectedAssets> {
        let zip = self
            .assets
            .iter()
            .find(|a| a.name == WINDOWS_PACKAGE)
            .cloned()
            .ok_or_else(|| {
                UpdateError::MissingAsset(format!(
                    "release {} does not contain the expected asset '{WINDOWS_PACKAGE}'",
                    self.tag_name
                ))
            })?;
        let checksum = self
            .assets
            .iter()
            .find(|a| a.name == CHECKSUM_ASSET)
            .cloned()
            .ok_or_else(|| {
                UpdateError::MissingAsset(format!(
                    "release {} does not contain the expected checksum asset '{CHECKSUM_ASSET}'",
                    self.tag_name
                ))
            })?;
        validate_download_url(&zip.browser_download_url, WINDOWS_PACKAGE)?;
        validate_download_url(&checksum.browser_download_url, CHECKSUM_ASSET)?;
        let zip_digest = zip.digest.clone();
        Ok(SelectedAssets {
            zip,
            checksum,
            zip_digest,
        })
    }
}

/// Public GitHub download URLs must use HTTPS; an empty or malformed URL is
/// rejected before any network request is attempted.
pub fn validate_download_url(url: &str, asset_name: &str) -> Result<()> {
    if url.trim() != url || url.is_empty() {
        return Err(UpdateError::InvalidRelease(format!(
            "asset '{asset_name}' has an empty or malformed download URL"
        )));
    }
    let parsed = reqwest::Url::parse(url).map_err(|_| {
        UpdateError::InvalidRelease(format!("asset '{asset_name}' has an invalid download URL"))
    })?;
    if parsed.scheme() != "https" {
        return Err(UpdateError::InvalidRelease(format!(
            "asset '{asset_name}' does not use HTTPS"
        )));
    }
    Ok(())
}

pub fn parse_release_json(body: &str) -> Result<Release> {
    serde_json::from_str(body).map_err(|e| {
        UpdateError::Api(format!(
            "GitHub returned release metadata that could not be read ({e})"
        ))
    })
}

fn api_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(API_TIMEOUT)
        .build()
        .map_err(|e| UpdateError::Network(format!("could not create the HTTP client ({e})")))
}

/// Fetch and validate the latest published (non-prerelease) release.
pub fn fetch_latest_release() -> Result<Release> {
    let client = api_client()?;
    let response = client
        .get(LATEST_RELEASE_URL)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", USER_AGENT)
        .send()
        .map_err(|e| {
            UpdateError::Network(format!(
                "could not reach GitHub Releases at {LATEST_RELEASE_URL} ({e})"
            ))
        })?;

    let status = response.status();
    let remaining = response
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if !status.is_success() {
        return Err(api_error(status.as_u16(), remaining.as_deref()));
    }
    let text = response.text().map_err(|e| {
        UpdateError::Network(format!(
            "failed while reading the GitHub API response ({e})"
        ))
    })?;
    parse_release_json(&text)
}

/// Translate a non-success GitHub API response into an actionable error.
fn api_error(status: u16, rate_limit_remaining: Option<&str>) -> UpdateError {
    if (status == 403 || status == 429) && rate_limit_remaining == Some("0") {
        return UpdateError::RateLimited;
    }
    match status {
        404 => UpdateError::Api(format!(
            "no published release was found for {REPOSITORY} (HTTP 404)"
        )),
        403 | 429 => UpdateError::Api(format!(
            "GitHub refused the release request (HTTP {status}); the API may be rate limited"
        )),
        _ => UpdateError::Api(format!(
            "GitHub Releases returned HTTP {status} for {REPOSITORY}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release_json(assets: &str) -> String {
        format!(
            r#"{{
                "tag_name": "v0.1.1",
                "html_url": "https://github.com/vishwassharma38/Akari-Sweeps/releases/tag/v0.1.1",
                "body": "Release notes",
                "assets": [{assets}]
            }}"#
        )
    }

    fn asset(name: &str, digest: Option<&str>) -> String {
        let digest_field = match digest {
            Some(d) => format!(r#", "digest": "{d}""#),
            None => String::new(),
        };
        format!(
            r#"{{
                "name": "{name}",
                "browser_download_url": "https://github.com/vishwassharma38/Akari-Sweeps/releases/download/v0.1.1/{name}"
                {digest_field}
            }}"#
        )
    }

    fn assets(extra: &[(&str, Option<&str>)]) -> String {
        let mut all = vec![
            asset(
                WINDOWS_PACKAGE,
                Some("sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
            ),
            asset(CHECKSUM_ASSET, None),
        ];
        for (name, digest) in extra {
            all.push(asset(name, *digest));
        }
        all.join(",")
    }

    #[test]
    fn expected_assets_are_selected_exactly() {
        let release = parse_release_json(&release_json(&assets(&[
            ("unrelated.bin", None),
            ("akari-sweeps-windows-x86_64.zip.sig", None),
        ])))
        .unwrap();
        let selected = release.windows_assets().unwrap();
        assert_eq!(selected.zip.name, WINDOWS_PACKAGE);
        assert_eq!(selected.checksum.name, CHECKSUM_ASSET);
        assert!(
            selected
                .zip_digest
                .as_deref()
                .unwrap()
                .starts_with("sha256:")
        );
        assert_eq!(release.version().unwrap().to_string(), "0.1.1");
    }

    #[test]
    fn unrelated_assets_are_ignored_even_when_listed_first() {
        let json = release_json(&assets(&[("first-asset.exe", None), ("another.zip", None)]));
        let release = parse_release_json(&json).unwrap();
        assert_eq!(release.windows_assets().unwrap().zip.name, WINDOWS_PACKAGE);
    }

    #[test]
    fn missing_zip_is_reported() {
        let only_checksum = asset(CHECKSUM_ASSET, None);
        let release = parse_release_json(&release_json(&only_checksum)).unwrap();
        let err = release.windows_assets().unwrap_err();
        assert!(err.to_string().contains(WINDOWS_PACKAGE));
        assert!(matches!(err, UpdateError::MissingAsset(_)));
    }

    #[test]
    fn missing_checksum_is_reported() {
        let only_zip = asset(WINDOWS_PACKAGE, None);
        let release = parse_release_json(&release_json(&only_zip)).unwrap();
        let err = release.windows_assets().unwrap_err();
        assert!(err.to_string().contains(CHECKSUM_ASSET));
        assert!(matches!(err, UpdateError::MissingAsset(_)));
    }

    #[test]
    fn malformed_release_metadata_is_rejected() {
        assert!(parse_release_json("not json").is_err());
        assert!(parse_release_json("{}").is_err());
        assert!(parse_release_json(r#"{"tag_name":"v1.0.0"}"#).is_err());
    }

    #[test]
    fn empty_or_missing_optional_fields_are_safe() {
        let json = r#"{
            "tag_name": "v1.0.0",
            "html_url": "https://example.com/release",
            "assets": []
        }"#;
        let release = parse_release_json(json).unwrap();
        assert!(release.body.is_none());
        assert!(release.windows_assets().is_err());
    }

    #[test]
    fn invalid_download_urls_are_rejected() {
        for url in ["", "http://example.com/file.zip", "not a url", " ftp://x/y"] {
            assert!(
                validate_download_url(url, WINDOWS_PACKAGE).is_err(),
                "{url:?} must be rejected"
            );
        }
        assert!(validate_download_url("https://example.com/file.zip", WINDOWS_PACKAGE).is_ok());
    }

    #[test]
    fn http_errors_are_classified() {
        assert!(matches!(api_error(404, None), UpdateError::Api(_)));
        assert!(matches!(
            api_error(403, Some("0")),
            UpdateError::RateLimited
        ));
        assert!(matches!(api_error(500, None), UpdateError::Api(_)));
    }

    #[test]
    fn rate_limit_error_is_user_friendly() {
        assert!(
            api_error(429, Some("0"))
                .to_string()
                .to_lowercase()
                .contains("rate limit")
        );
    }
}
