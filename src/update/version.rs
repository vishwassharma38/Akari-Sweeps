//! Version discovery and semantic comparison.
//!
//! `Cargo.toml` is the single source of truth: the installed version comes
//! from `env!("CARGO_PKG_VERSION")`, exactly like the About screen and the
//! `--version` output. GitHub tags are parsed as semantic versions, with a
//! single leading `v` normalized away.

use super::{Result, UpdateError};
use semver::Version;

/// Installed application version, compiled in from Cargo.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Parsed installed version. Cargo guarantees a valid semver package
/// version, so this cannot fail for a normal build.
pub fn installed_version() -> Version {
    Version::parse(CURRENT_VERSION).expect("CARGO_PKG_VERSION must be valid semantic versioning")
}

/// Parse a GitHub release tag such as `v0.1.1`. Accepts one leading `v` or
/// `V`; anything else must already be a valid semantic version.
pub fn parse_release_tag(tag: &str) -> Result<Version> {
    let trimmed = tag.trim();
    if trimmed.is_empty() {
        return Err(UpdateError::InvalidVersion(
            "the latest release has an empty tag".to_string(),
        ));
    }
    let without_prefix = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);
    if without_prefix.is_empty() {
        return Err(UpdateError::InvalidVersion(format!(
            "release tag '{trimmed}' does not contain a version"
        )));
    }
    Version::parse(without_prefix).map_err(|e| {
        UpdateError::InvalidVersion(format!(
            "release tag '{trimmed}' is not a valid semantic version ({e})"
        ))
    })
}

/// Whether `latest` is strictly newer than `installed`. Semantic version
/// ordering is used; prerelease/build metadata follow semver rules.
pub fn is_newer(installed: &Version, latest: &Version) -> bool {
    latest > installed
}

/// Human-readable result of comparing the installed and published versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    UpToDate,
    UpdateAvailable,
    NewerInstalled,
}

pub fn availability(installed: &Version, latest: &Version) -> Availability {
    if latest > installed {
        Availability::UpdateAvailable
    } else if latest < installed {
        Availability::NewerInstalled
    } else {
        Availability::UpToDate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn current_version_is_compiled_from_cargo_and_valid() {
        assert!(!CURRENT_VERSION.is_empty());
        assert_eq!(installed_version(), v(CURRENT_VERSION));
        assert_eq!(CURRENT_VERSION, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn identical_versions_are_up_to_date() {
        assert!(!is_newer(&v("0.1.0"), &v("0.1.0")));
        assert_eq!(
            availability(&v("0.1.0"), &v("0.1.0")),
            Availability::UpToDate
        );
    }

    #[test]
    fn newer_latest_is_an_update() {
        assert!(is_newer(&v("0.1.0"), &v("0.1.1")));
        assert_eq!(
            availability(&v("0.1.0"), &v("0.1.1")),
            Availability::UpdateAvailable
        );
    }

    #[test]
    fn older_latest_is_not_an_update() {
        assert!(!is_newer(&v("0.1.1"), &v("0.1.0")));
        assert_eq!(
            availability(&v("0.1.1"), &v("0.1.0")),
            Availability::NewerInstalled
        );
    }

    #[test]
    fn multi_digit_minor_is_compared_numerically() {
        // String comparison would order "0.9.9" after "0.10.0"; semver does not.
        assert!(is_newer(&v("0.9.9"), &v("0.10.0")));
        assert!(!is_newer(&v("0.10.0"), &v("0.9.9")));
    }

    #[test]
    fn leading_v_tags_are_normalized() {
        assert_eq!(parse_release_tag("v0.1.1").unwrap(), v("0.1.1"));
        assert_eq!(parse_release_tag("V1.2.3").unwrap(), v("1.2.3"));
        assert_eq!(parse_release_tag("  v0.2.0  ").unwrap(), v("0.2.0"));
        assert_eq!(parse_release_tag("1.0.0").unwrap(), v("1.0.0"));
    }

    #[test]
    fn prerelease_ordering_follows_semver() {
        assert!(is_newer(&v("1.0.0-beta.1"), &v("1.0.0")));
        assert!(!is_newer(&v("1.0.0"), &v("1.0.0-beta.1")));
        assert!(is_newer(&v("1.0.0-beta.1"), &v("1.0.0-beta.2")));
    }

    #[test]
    fn invalid_tags_are_rejected() {
        for tag in [
            "",
            " ",
            "v",
            "V",
            "vv1.0.0",
            "release-1.0.0",
            "1.2",
            "1.2.3.4",
        ] {
            assert!(parse_release_tag(tag).is_err(), "{tag:?} must be rejected");
        }
    }
}
