//! Automatic update-check cache.
//!
//! The cache lives in the per-user local application data directory, not in
//! the installation directory, so custom installations share the same
//! policy. Manual `akari update` / `akari update --check` never consult it.
//!
//! A successful release API response is cached for 24 hours. A failed check
//! (no published release yet, network failure, server error, invalid payload,
//! rate limiting) is cached for a much shorter retry window so a newly
//! published release is discovered promptly instead of being missed for a
//! full day. The on-disk format remains backwards compatible: old cache files
//! only contain `last_checked_unix_secs`, which is treated as the last
//! successful check.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Normal interval after a successful release API response.
pub const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// Short retry interval after a failed check. Keeps automatic requests
/// respectful while still noticing an initial release or a transient outage
/// quickly.
pub const RETRY_INTERVAL_SECS: u64 = 60 * 60;

/// Lower bound applied to a numeric `Retry-After` header. GitHub rate-limit
/// responses must not make the updater hammer the API.
pub const MIN_RATE_LIMIT_RETRY_SECS: u64 = 60;

/// Upper bound applied to a numeric `Retry-After` header. A server cannot
/// suppress automatic checks for an unbounded amount of time.
pub const MAX_RATE_LIMIT_RETRY_SECS: u64 = 6 * 60 * 60;

/// Per-user directory for updater state that must be independent of the
/// installation directory. `%LOCALAPPDATA%\Akari-Sweeps` on Windows; the
/// temp directory is a safe fallback for tests/non-Windows hosts.
pub fn user_data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("Akari-Sweeps")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CacheFile {
    /// Last successful release API check. `0` means "never".
    #[serde(default)]
    last_checked_unix_secs: u64,
    /// When the next attempt is allowed after a failed check. Absent in
    /// older cache files and after a successful check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    next_check_unix_secs: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CacheStore {
    path: PathBuf,
}

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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

    fn read(&self) -> Option<CacheFile> {
        let bytes = fs::read(&self.path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Last successful check, or `None` for missing/corrupt data.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn last_check_unix_secs(&self) -> Option<u64> {
        self.read()
            .filter(|cache| cache.last_checked_unix_secs != 0)
            .map(|cache| cache.last_checked_unix_secs)
    }

    /// Earliest time a failed check may be retried, if a failure was
    /// recorded.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn next_check_unix_secs(&self) -> Option<u64> {
        self.read().and_then(|cache| cache.next_check_unix_secs)
    }

    /// True when the automatic check may run. A pending retry takes
    /// precedence over the long success interval. Invalid data behaves like
    /// a missing cache (check now); future timestamps are treated as invalid
    /// so a skewed clock cannot suppress checks forever.
    pub fn should_check(&self, now_unix_secs: u64) -> bool {
        let Some(cache) = self.read() else {
            return true;
        };
        if let Some(next) = cache.next_check_unix_secs {
            return now_unix_secs >= next;
        }
        match cache.last_checked_unix_secs {
            0 => true,
            last if last > now_unix_secs => true,
            last => now_unix_secs.saturating_sub(last) >= CHECK_INTERVAL_SECS,
        }
    }

    /// Persist a successful check. This clears any pending short retry.
    pub fn record_success(&self, now_unix_secs: u64) -> std::io::Result<()> {
        self.write(&CacheFile {
            last_checked_unix_secs: now_unix_secs,
            next_check_unix_secs: None,
        })
    }

    /// Persist a failed check without overwriting the last successful
    /// timestamp. The success timestamp is retained so documentation and
    /// diagnostics remain accurate after a transient failure.
    pub fn record_failure(&self, now_unix_secs: u64, retry_after_secs: u64) -> std::io::Result<()> {
        let mut cache = self.read().unwrap_or_default();
        cache.next_check_unix_secs =
            Some(now_unix_secs.saturating_add(retry_after_secs.max(MIN_RATE_LIMIT_RETRY_SECS)));
        self.write(&cache)
    }

    /// Persist atomically (same-directory temp file + rename). Returns an
    /// error for callers that care; automatic checks deliberately ignore it
    /// so a cache problem never blocks startup.
    fn write(&self, cache: &CacheFile) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let payload = serde_json::to_vec_pretty(cache).map_err(std::io::Error::other)?;
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = self
            .path
            .with_extension(format!("tmp-{}-{sequence}", std::process::id()));
        fs::write(&temp, payload)?;
        if let Err(e) = fs::rename(&temp, &self.path) {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        Ok(())
    }

    pub fn record_success_best_effort(&self, now_unix_secs: u64) {
        let _ = self.record_success(now_unix_secs);
    }

    pub fn record_failure_best_effort(&self, now_unix_secs: u64, retry_after_secs: u64) {
        let _ = self.record_failure(now_unix_secs, retry_after_secs);
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
    fn recent_success_suppresses_automatic_request() {
        let cache = temp_cache("recent");
        cache.record_success(1_000_000).unwrap();
        assert!(!cache.should_check(1_000_000));
        assert!(!cache.should_check(1_000_000 + CHECK_INTERVAL_SECS - 1));
        assert_eq!(cache.next_check_unix_secs(), None);
    }

    #[test]
    fn expired_success_allows_another_request() {
        let cache = temp_cache("expired");
        cache.record_success(1_000_000).unwrap();
        assert!(cache.should_check(1_000_000 + CHECK_INTERVAL_SECS));
        assert!(cache.should_check(1_000_000 + CHECK_INTERVAL_SECS + 500));
    }

    #[test]
    fn failed_check_uses_the_short_retry_window() {
        let cache = temp_cache("retry");
        cache
            .record_failure(1_000_000, RETRY_INTERVAL_SECS)
            .unwrap();
        assert!(!cache.should_check(1_000_000));
        assert!(!cache.should_check(1_000_000 + RETRY_INTERVAL_SECS - 1));
        assert!(cache.should_check(1_000_000 + RETRY_INTERVAL_SECS));
        assert_eq!(cache.last_check_unix_secs(), None);
        assert_eq!(
            cache.next_check_unix_secs(),
            Some(1_000_000 + RETRY_INTERVAL_SECS)
        );
    }

    #[test]
    fn failure_preserves_a_previous_success_timestamp() {
        let cache = temp_cache("preserve");
        cache.record_success(1_000_000).unwrap();
        cache
            .record_failure(1_000_100, RETRY_INTERVAL_SECS)
            .unwrap();
        assert_eq!(cache.last_check_unix_secs(), Some(1_000_000));
        assert!(!cache.should_check(1_000_100 + RETRY_INTERVAL_SECS - 1));
        assert!(cache.should_check(1_000_100 + RETRY_INTERVAL_SECS));
    }

    #[test]
    fn success_clears_a_pending_retry() {
        let cache = temp_cache("clear");
        cache
            .record_failure(1_000_000, RETRY_INTERVAL_SECS)
            .unwrap();
        cache.record_success(1_000_100).unwrap();
        assert_eq!(cache.next_check_unix_secs(), None);
        assert_eq!(cache.last_check_unix_secs(), Some(1_000_100));
    }

    #[test]
    fn old_cache_format_remains_compatible() {
        let cache = temp_cache("legacy");
        fs::create_dir_all(cache.path().parent().unwrap()).unwrap();
        fs::write(cache.path(), b"{\"last_checked_unix_secs\": 1000000}").unwrap();
        assert_eq!(cache.last_check_unix_secs(), Some(1_000_000));
        assert!(!cache.should_check(1_000_000 + CHECK_INTERVAL_SECS - 1));
        assert!(cache.should_check(1_000_000 + CHECK_INTERVAL_SECS));
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
        cache.record_success(2_000_000).unwrap();
        assert!(cache.should_check(1_000_000));
    }

    #[test]
    fn recording_is_atomic_and_overwrites_cleanly() {
        let cache = temp_cache("record");
        cache.record_success(1_000_000).unwrap();
        assert_eq!(cache.last_check_unix_secs(), Some(1_000_000));
        cache.record_success(1_000_001).unwrap();
        assert_eq!(cache.last_check_unix_secs(), Some(1_000_001));
        // No stray temp files remain next to the cache.
        let siblings: Vec<_> = fs::read_dir(cache.path().parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(siblings.len(), 1, "{siblings:?}");
    }

    #[test]
    fn cache_write_failure_is_reported_but_does_not_corrupt_other_files() {
        let root = temp_cache("blocked").path().parent().unwrap().to_path_buf();
        fs::create_dir_all(&root).unwrap();
        let blocker = root.join("not-a-directory");
        fs::write(&blocker, b"file").unwrap();
        let cache = CacheStore::new(blocker.join("update-check.json"));
        assert!(cache.record_success(1_000_000).is_err());
        assert_eq!(fs::read(&blocker).unwrap(), b"file");
    }

    #[test]
    fn manual_check_bypass_is_structural() {
        // Manual CLI paths call `github::fetch_latest_release` directly and
        // never consume `CacheStore`; this test documents the invariant by
        // proving a suppressing cache exists without affecting that path.
        let cache = temp_cache("manual");
        cache.record_success(now_unix_secs()).unwrap();
        assert!(!cache.should_check(now_unix_secs()));
        // The manual path has no cache dependency at all (see update::mod).
    }
}
