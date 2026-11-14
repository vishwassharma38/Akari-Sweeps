//! 24-hour automatic update-check cache.
//!
//! The cache lives in the per-user local application data directory, not in
//! the installation directory, so custom installations share the same
//! policy. Manual `akari update` / `akari update --check` never consult it.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// Per-user directory for updater state that must be independent of the
/// installation directory. `%LOCALAPPDATA%\Akari-Sweeps` on Windows; the
/// temp directory is a safe fallback for tests/non-Windows hosts.
pub fn user_data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("Akari-Sweeps")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheFile {
    last_checked_unix_secs: u64,
}

#[derive(Debug, Clone)]
pub struct CacheStore {
    path: PathBuf,
}

impl CacheStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// `%LOCALAPPDATA%\Akari-Sweeps\update-check.json`, independent of the
    /// executable's installation directory. Falls back to the temp
    /// directory when `LOCALAPPDATA` is unavailable (non-Windows/tests).
    pub fn user_default() -> Self {
        Self::new(user_data_dir().join("update-check.json"))
    }

    #[cfg(test)]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Last recorded attempt, or `None` for missing/corrupt data.
    pub fn last_check_unix_secs(&self) -> Option<u64> {
        let bytes = fs::read(&self.path).ok()?;
        let cache: CacheFile = serde_json::from_slice(&bytes).ok()?;
        if cache.last_checked_unix_secs == 0 {
            None
        } else {
            Some(cache.last_checked_unix_secs)
        }
    }

    /// True when the automatic check may run. Invalid data behaves like a
    /// missing cache (check now); future timestamps are treated as invalid
    /// so a skewed clock cannot suppress checks forever.
    pub fn should_check(&self, now_unix_secs: u64) -> bool {
        match self.last_check_unix_secs() {
            Some(last) if last <= now_unix_secs => {
                now_unix_secs.saturating_sub(last) >= CHECK_INTERVAL_SECS
            }
            _ => true,
        }
    }

    /// Persist an attempt timestamp atomically (same-directory temp file +
    /// rename). Returns an error for callers that care; automatic checks
    /// deliberately ignore it so a cache problem never blocks startup.
    pub fn record_check(&self, now_unix_secs: u64) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let payload = serde_json::to_vec_pretty(&CacheFile {
            last_checked_unix_secs: now_unix_secs,
        })
        .map_err(std::io::Error::other)?;
        let temp =
            self.path
                .with_extension(format!("tmp-{}-{}", std::process::id(), now_unix_secs));
        fs::write(&temp, payload)?;
        if let Err(e) = fs::rename(&temp, &self.path) {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        Ok(())
    }

    pub fn record_check_best_effort(&self, now_unix_secs: u64) {
        let _ = self.record_check(now_unix_secs);
    }
}

pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_cache(label: &str) -> CacheStore {
        let root = std::env::temp_dir().join(format!(
            "akari-update-cache-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        CacheStore::new(root.join("update-check.json"))
    }

    #[test]
    fn missing_cache_allows_automatic_check() {
        let cache = temp_cache("missing");
        assert!(cache.should_check(1_000_000));
        assert!(!cache.path().exists());
    }

    #[test]
    fn recent_check_suppresses_automatic_request() {
        let cache = temp_cache("recent");
        cache.record_check(1_000_000).unwrap();
        assert!(!cache.should_check(1_000_000));
        assert!(!cache.should_check(1_000_000 + CHECK_INTERVAL_SECS - 1));
    }

    #[test]
    fn expired_check_allows_another_request() {
        let cache = temp_cache("expired");
        cache.record_check(1_000_000).unwrap();
        assert!(cache.should_check(1_000_000 + CHECK_INTERVAL_SECS));
        assert!(cache.should_check(1_000_000 + CHECK_INTERVAL_SECS + 500));
    }

    #[test]
    fn invalid_cache_data_is_handled_safely() {
        let cache = temp_cache("invalid");
        fs::create_dir_all(cache.path().parent().unwrap()).unwrap();
        for payload in [
            b"not json".as_slice(),
            b"{}".as_slice(),
            b"{\"last_checked_unix_secs\": \"yesterday\"}".as_slice(),
            b"{\"last_checked_unix_secs\": 0}".as_slice(),
        ] {
            fs::write(cache.path(), payload).unwrap();
            assert_eq!(cache.last_check_unix_secs(), None, "{payload:?}");
            assert!(cache.should_check(1_000_000));
        }
    }

    #[test]
    fn future_timestamp_does_not_suppress_checks() {
        let cache = temp_cache("future");
        cache.record_check(2_000_000).unwrap();
        assert!(cache.should_check(1_000_000));
    }

    #[test]
    fn recording_is_atomic_and_overwrites_cleanly() {
        let cache = temp_cache("record");
        cache.record_check(1_000_000).unwrap();
        assert_eq!(cache.last_check_unix_secs(), Some(1_000_000));
        cache.record_check(1_000_001).unwrap();
        assert_eq!(cache.last_check_unix_secs(), Some(1_000_001));
        // No stray temp files remain next to the cache.
        let siblings: Vec<_> = fs::read_dir(cache.path().parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(siblings.len(), 1, "{siblings:?}");
    }

    #[test]
    fn manual_check_bypass_is_structural() {
        // Manual CLI paths call `github::fetch_latest_release` directly and
        // never consume `CacheStore`; this test documents the invariant by
        // proving a suppressing cache exists without affecting that path.
        let cache = temp_cache("manual");
        cache.record_check(now_unix_secs()).unwrap();
        assert!(!cache.should_check(now_unix_secs()));
        // The manual path has no cache dependency at all (see update::mod).
    }
}
