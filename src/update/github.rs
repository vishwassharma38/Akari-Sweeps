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

pub const REPOSITORY_OWNER: &str = "vishwassharma38";
pub const REPOSITORY_NAME: &str = "Akari-Sweeps";
pub const REPOSITORY: &str = "vishwassharma38/Akari-Sweeps";
pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/vishwassharma38/Akari-Sweeps/releases/latest";
pub const USER_AGENT: &str = "Akari-Sweeps";

/// The only host accepted for the initial release-asset URL.
pub const GITHUB_HOST: &str = "github.com";

/// GitHub-owned hosts used by the release-asset download redirect. The
/// updater follows redirects manually and never trusts an arbitrary host.
pub const RELEASE_ASSET_HOSTS: [&str; 2] = [
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];

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

/// Validate the original release asset URL. Only an HTTPS URL on
/// `github.com` for the exact repository, release-download path, and expected
/// asset filename is accepted. Parsed components are compared explicitly; no
/// substring or suffix check is used.
pub fn validate_download_url(url: &str, asset_name: &str) -> Result<()> {
    let parsed = parse_asset_url(url, asset_name)?;
    validate_https_transport(&parsed, asset_name)?;

    if parsed.host_str() != Some(GITHUB_HOST) {
        return Err(UpdateError::InvalidRelease(format!(
            "asset '{asset_name}' is not hosted on {GITHUB_HOST}"
        )));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(UpdateError::InvalidRelease(format!(
            "asset '{asset_name}' has an unexpected query or fragment"
        )));
    }

    let segments: Vec<&str> = parsed
        .path_segments()
        .map(|segments| segments.collect())
        .unwrap_or_default();
    if segments.len() != 6
        || segments[0] != REPOSITORY_OWNER
        || segments[1] != REPOSITORY_NAME
        || segments[2] != "releases"
        || segments[3] != "download"
    {
        return Err(UpdateError::InvalidRelease(format!(
            "asset '{asset_name}' is not a release download for {REPOSITORY}"
        )));
    }
    let tag = segments[4];
    if tag.is_empty() || tag == "." || tag == ".." {
        return Err(UpdateError::InvalidRelease(format!(
            "asset '{asset_name}' has an invalid release tag path"
        )));
    }
    if segments[5] != asset_name {
        return Err(UpdateError::InvalidRelease(format!(
            "asset URL does not end with the expected filename '{asset_name}'"
        )));
    }
    Ok(())
}

/// Validate one redirect destination before it is requested.
///
/// GitHub may redirect a release asset to its signed CDN host. Any redirect
/// back to `github.com` must still identify this repository and the expected
/// asset; CDN redirects must use HTTPS, no credentials, and no unexpected
/// port. Signed query parameters on the CDN URL are preserved.
pub fn validate_redirect_url(url: &str, asset_name: &str) -> Result<()> {
    let parsed = parse_asset_url(url, asset_name)?;
    validate_https_transport(&parsed, asset_name)?;

    match parsed.host_str() {
        Some(GITHUB_HOST) => validate_download_url(url, asset_name),
        Some(host) if RELEASE_ASSET_HOSTS.contains(&host) => {
            if parsed.fragment().is_some() {
                return Err(UpdateError::InvalidRelease(format!(
                    "asset '{asset_name}' redirect points to a URL with a fragment"
                )));
            }
            Ok(())
        }
        Some(host) => Err(UpdateError::InvalidRelease(format!(
            "asset '{asset_name}' redirect points to an untrusted host '{host}'"
        ))),
        None => Err(UpdateError::InvalidRelease(format!(
            "asset '{asset_name}' redirect has no host"
        ))),
    }
}

fn parse_asset_url(url: &str, asset_name: &str) -> Result<reqwest::Url> {
    if url.is_empty() || url.trim() != url {
        return Err(UpdateError::InvalidRelease(format!(
            "asset '{asset_name}' has an empty or malformed download URL"
        )));
    }
    reqwest::Url::parse(url).map_err(|_| {
        UpdateError::InvalidRelease(format!("asset '{asset_name}' has an invalid download URL"))
    })
}

fn validate_https_transport(parsed: &reqwest::Url, asset_name: &str) -> Result<()> {
    if parsed.scheme() != "https" {
        return Err(UpdateError::InvalidRelease(format!(
            "asset '{asset_name}' does not use HTTPS"
        )));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(UpdateError::InvalidRelease(format!(
            "asset '{asset_name}' URL contains credentials"
        )));
    }
    match parsed.port() {
        None | Some(443) => Ok(()),
        Some(port) => Err(UpdateError::InvalidRelease(format!(
            "asset '{asset_name}' URL uses unexpected port {port}"
        ))),
    }
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
    let retry_after_secs = parse_retry_after(response.headers());
    if !status.is_success() {
        return Err(api_error(
            status.as_u16(),
            remaining.as_deref(),
            retry_after_secs,
        ));
    }
    let text = response.text().map_err(|e| {
        UpdateError::Network(format!(
            "failed while reading the GitHub API response ({e})"
        ))
    })?;
    parse_release_json(&text)
}

/// Parse a numeric `Retry-After` header. HTTP-date values are deliberately
/// ignored; the caller then falls back to the normal short retry interval.
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

/// Translate a non-success GitHub API response into an actionable error.
fn api_error(
    status: u16,
    rate_limit_remaining: Option<&str>,
    retry_after_secs: Option<u64>,
) -> UpdateError {
    if status == 429 || ((status == 403) && rate_limit_remaining == Some("0")) {
        return UpdateError::RateLimited { retry_after_secs };
    }
    match status {
        // `releases/latest` returns 404 until the first non-prerelease
        // release exists. This is a distinct, retryable state, not a
        // successful "no newer release" answer.
        404 => UpdateError::NoRelease,
        403 => UpdateError::Api(format!(
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

    fn asset_url(tag: &str, name: &str) -> String {
        format!(
            "https://github.com/{REPOSITORY_OWNER}/{REPOSITORY_NAME}/releases/download/{tag}/{name}"
        )
    }

    #[test]
    fn expected_repository_release_asset_url_is_accepted() {
        let url = asset_url("v0.1.1", WINDOWS_PACKAGE);
        assert!(validate_download_url(&url, WINDOWS_PACKAGE).is_ok());
        assert!(
            validate_download_url(&asset_url("v0.1.1", CHECKSUM_ASSET), CHECKSUM_ASSET).is_ok()
        );
    }

    #[test]
    fn unrelated_urls_are_rejected() {
        let cases = [
            String::new(),
            "not a url".to_string(),
            " ftp://x/y".to_string(),
            "http://github.com/vishwassharma38/Akari-Sweeps/releases/download/v1.0.0/akari-sweeps-windows-x86_64.zip".to_string(),
            "https://example.com/file.zip".to_string(),
            asset_url("v0.1.1", "other.zip"),
        ];
        for url in cases {
            assert!(
                validate_download_url(&url, WINDOWS_PACKAGE).is_err(),
                "{url:?} must be rejected"
            );
        }
    }

    #[test]
    fn other_repositories_and_lookalike_hosts_are_rejected() {
        let cases = [
            format!(
                "https://github.com/vishwassharma38/Other-Project/releases/download/v0.1.1/{WINDOWS_PACKAGE}"
            ),
            format!(
                "https://github.com/other-owner/Akari-Sweeps/releases/download/v0.1.1/{WINDOWS_PACKAGE}"
            ),
            format!(
                "https://github.com.evil.example/vishwassharma38/Akari-Sweeps/releases/download/v0.1.1/{WINDOWS_PACKAGE}"
            ),
            format!(
                "https://evilgithub.com/vishwassharma38/Akari-Sweeps/releases/download/v0.1.1/{WINDOWS_PACKAGE}"
            ),
            format!(
                "https://github.com/vishwassharma38/Akari-Sweeps.evil/releases/download/v0.1.1/{WINDOWS_PACKAGE}"
            ),
        ];
        for url in cases {
            assert!(
                validate_download_url(&url, WINDOWS_PACKAGE).is_err(),
                "{url:?} must be rejected"
            );
        }
    }

    #[test]
    fn unexpected_paths_ports_and_credentials_are_rejected() {
        let cases = [
            format!(
                "https://github.com/vishwassharma38/Akari-Sweeps/archive/refs/tags/v0.1.1/{WINDOWS_PACKAGE}"
            ),
            format!(
                "https://github.com/vishwassharma38/Akari-Sweeps/releases/latest/{WINDOWS_PACKAGE}"
            ),
            format!(
                "https://github.com:8443/vishwassharma38/Akari-Sweeps/releases/download/v0.1.1/{WINDOWS_PACKAGE}"
            ),
            format!(
                "https://user:pass@github.com/vishwassharma38/Akari-Sweeps/releases/download/v0.1.1/{WINDOWS_PACKAGE}"
            ),
            format!(
                "https://github.com/vishwassharma38/Akari-Sweeps/releases/download/v0.1.1/{WINDOWS_PACKAGE}?download=1"
            ),
        ];
        for url in cases {
            assert!(
                validate_download_url(&url, WINDOWS_PACKAGE).is_err(),
                "{url:?} must be rejected"
            );
        }
    }

    #[test]
    fn github_release_asset_redirects_are_accepted_with_signed_query() {
        for host in RELEASE_ASSET_HOSTS {
            let url = format!(
                "https://{host}/github-production-release-asset/1234/abcd?sp=r&sig=abc%2Fdef&se=2030-01-01"
            );
            assert!(
                validate_redirect_url(&url, WINDOWS_PACKAGE).is_ok(),
                "{host} must be accepted"
            );
        }
        // A redirect that stays on GitHub must still point at the exact
        // release asset.
        assert!(
            validate_redirect_url(&asset_url("v0.1.1", WINDOWS_PACKAGE), WINDOWS_PACKAGE).is_ok()
        );
    }

    #[test]
    fn malicious_or_unexpected_redirects_are_rejected() {
        let cases = [
            "https://evil.example/asset.zip".to_string(),
            "http://release-assets.githubusercontent.com/asset.zip".to_string(),
            "https://release-assets.githubusercontent.com:8443/asset.zip".to_string(),
            "https://user:pass@release-assets.githubusercontent.com/asset.zip".to_string(),
            "https://github.com.evil.example/vishwassharma38/Akari-Sweeps/releases/download/v0.1.1/akari-sweeps-windows-x86_64.zip".to_string(),
            format!("https://github.com/other-owner/Akari-Sweeps/releases/download/v0.1.1/{WINDOWS_PACKAGE}"),
            asset_url("v0.1.1", "other.zip"),
        ];
        for url in cases {
            assert!(
                validate_redirect_url(&url, WINDOWS_PACKAGE).is_err(),
                "{url:?} must be rejected"
            );
        }
    }

    #[test]
    fn http_errors_are_classified() {
        assert!(matches!(api_error(404, None, None), UpdateError::NoRelease));
        assert!(matches!(
            api_error(403, Some("0"), Some(120)),
            UpdateError::RateLimited {
                retry_after_secs: Some(120)
            }
        ));
        assert!(matches!(
            api_error(429, None, None),
            UpdateError::RateLimited {
                retry_after_secs: None
            }
        ));
        assert!(matches!(api_error(500, None, None), UpdateError::Api(_)));
    }

    #[test]
    fn rate_limit_error_is_user_friendly() {
        assert!(
            api_error(429, Some("0"), None)
                .to_string()
                .to_lowercase()
                .contains("rate limit")
        );
    }
}
