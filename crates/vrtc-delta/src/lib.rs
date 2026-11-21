//! Native VRTc content change detection and bounded version history.
//!
//! SHA-256 fingerprints identify content changes; line diffs are generated
//! only within configured bounds. This is a local in-memory history engine.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use url::Url;
use vrtc_core::{ErrorCategory, VrtCError};

#[derive(Debug, Clone)]
pub struct DeltaConfig {
    pub max_tracked_urls: usize,
    pub max_versions_per_url: usize,
    pub max_snapshot_bytes: usize,
    pub max_diff_lines: usize,
}

impl Default for DeltaConfig {
    fn default() -> Self {
        Self {
            max_tracked_urls: 100_000,
            max_versions_per_url: 20,
            max_snapshot_bytes: 8 * 1024 * 1024,
            max_diff_lines: 20_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Created,
    Modified,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSnapshot {
    pub url: String,
    pub version: u64,
    pub title: String,
    pub description: String,
    pub content: String,
    pub content_hash: String,
    pub first_seen_at: SystemTime,
    pub last_seen_at: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaRecord {
    pub url: String,
    pub version: u64,
    pub kind: ChangeKind,
    pub previous_hash: Option<String>,
    pub current_hash: String,
    pub added_lines: usize,
    pub removed_lines: usize,
    pub metadata_changed: bool,
    pub diff_truncated: bool,
    pub observed_at: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeltaStats {
    pub tracked_urls: usize,
    pub retained_versions: usize,
}

#[derive(Default)]
struct DeltaState {
    histories: HashMap<String, Vec<DocumentSnapshot>>,
}

pub struct DeltaEngine {
    config: DeltaConfig,
    state: Mutex<DeltaState>,
}

impl DeltaEngine {
    pub fn new(config: DeltaConfig) -> Result<Self, VrtCError> {
        if config.max_tracked_urls == 0
            || config.max_versions_per_url == 0
            || config.max_snapshot_bytes == 0
            || config.max_diff_lines == 0
        {
            return Err(delta_error(
                "INVALID_DELTA_CONFIG",
                "URL, version, snapshot, and diff limits must be positive",
                ErrorCategory::Validation,
                false,
            ));
        }
        Ok(Self {
            config,
            state: Mutex::new(DeltaState::default()),
        })
    }

    /// Record a normalized observation and classify it as created, modified,
    /// or unchanged. Unchanged observations update last-seen time without
    /// consuming another version slot.
    pub fn record_snapshot(
        &self,
        raw_url: &str,
        title: &str,
        description: &str,
        content: &str,
        observed_at: SystemTime,
    ) -> Result<DeltaRecord, VrtCError> {
        let url = canonicalize_url(raw_url)?;
        let byte_size = title
            .len()
            .saturating_add(description.len())
            .saturating_add(content.len());
        if byte_size > self.config.max_snapshot_bytes {
            return Err(delta_error(
                "DELTA_SNAPSHOT_TOO_LARGE",
                "Snapshot exceeds the configured byte limit",
                ErrorCategory::ResourceExhausted,
                false,
            ));
        }

        let current_hash = fingerprint(title, description, content);
        let mut state = self.lock_state()?;
        let history_exists = state.histories.contains_key(&url);
        if !history_exists && state.histories.len() >= self.config.max_tracked_urls {
            return Err(delta_error(
                "DELTA_CAPACITY_REACHED",
                "The delta engine has reached its tracked URL capacity",
                ErrorCategory::ResourceExhausted,
                false,
            ));
        }

        let history = state.histories.entry(url.clone()).or_default();
        if let Some(previous) = history.last_mut() {
            if previous.content_hash == current_hash {
                previous.last_seen_at = observed_at;
                return Ok(DeltaRecord {
                    url,
                    version: previous.version,
                    kind: ChangeKind::Unchanged,
                    previous_hash: Some(previous.content_hash.clone()),
                    current_hash,
                    added_lines: 0,
                    removed_lines: 0,
                    metadata_changed: false,
                    diff_truncated: false,
                    observed_at,
                });
            }

            let previous_hash = previous.content_hash.clone();
            let metadata_changed = previous.title != title || previous.description != description;
            let (added_lines, removed_lines, diff_truncated) =
                calculate_diff(&previous.content, content, self.config.max_diff_lines);
            let version = previous.version.saturating_add(1);
            history.push(DocumentSnapshot {
                url: url.clone(),
                version,
                title: title.to_owned(),
                description: description.to_owned(),
                content: content.to_owned(),
                content_hash: current_hash.clone(),
                first_seen_at: observed_at,
                last_seen_at: observed_at,
            });
            trim_history(history, self.config.max_versions_per_url);
            return Ok(DeltaRecord {
                url,
                version,
                kind: ChangeKind::Modified,
                previous_hash: Some(previous_hash),
                current_hash,
                added_lines,
                removed_lines,
                metadata_changed,
                diff_truncated,
                observed_at,
            });
        }

        history.push(DocumentSnapshot {
            url: url.clone(),
            version: 1,
            title: title.to_owned(),
            description: description.to_owned(),
            content: content.to_owned(),
            content_hash: current_hash.clone(),
            first_seen_at: observed_at,
            last_seen_at: observed_at,
        });
        Ok(DeltaRecord {
            url,
            version: 1,
            kind: ChangeKind::Created,
            previous_hash: None,
            current_hash,
            added_lines: if content.lines().count() > self.config.max_diff_lines {
                0
            } else {
                content.lines().count()
            },
            removed_lines: 0,
            metadata_changed: true,
            diff_truncated: content.lines().count() > self.config.max_diff_lines,
            observed_at,
        })
    }

    /// Return the newest retained snapshot for a URL.
    pub fn latest(&self, raw_url: &str) -> Result<Option<DocumentSnapshot>, VrtCError> {
        let url = canonicalize_url(raw_url)?;
        let state = self.lock_state()?;
        Ok(state
            .histories
            .get(&url)
            .and_then(|history| history.last())
            .cloned())
    }

    /// Return retained versions in chronological order, bounded by limit.
    pub fn history(&self, raw_url: &str, limit: usize) -> Result<Vec<DocumentSnapshot>, VrtCError> {
        if limit == 0 || limit > self.config.max_versions_per_url {
            return Err(delta_error(
                "INVALID_DELTA_HISTORY_LIMIT",
                "History limit is outside the configured range",
                ErrorCategory::Validation,
                false,
            ));
        }
        let url = canonicalize_url(raw_url)?;
        let state = self.lock_state()?;
        let Some(history) = state.histories.get(&url) else {
            return Ok(Vec::new());
        };
        let start = history.len().saturating_sub(limit);
        Ok(history[start..].to_vec())
    }

    pub fn stats(&self) -> Result<DeltaStats, VrtCError> {
        let state = self.lock_state()?;
        Ok(DeltaStats {
            tracked_urls: state.histories.len(),
            retained_versions: state.histories.values().map(Vec::len).sum(),
        })
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, DeltaState>, VrtCError> {
        self.state.lock().map_err(|_| {
            delta_error(
                "DELTA_LOCK_POISONED",
                "The delta engine state is temporarily unavailable",
                ErrorCategory::Internal,
                true,
            )
        })
    }
}

fn trim_history(history: &mut Vec<DocumentSnapshot>, maximum: usize) {
    if history.len() > maximum {
        let excess = history.len() - maximum;
        history.drain(0..excess);
    }
}

fn calculate_diff(old: &str, new: &str, max_lines: usize) -> (usize, usize, bool) {
    if old.lines().count().max(new.lines().count()) > max_lines {
        return (0, 0, true);
    }
    let diff = TextDiff::from_lines(old, new);
    let mut added = 0;
    let mut removed = 0;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added += 1,
            ChangeTag::Delete => removed += 1,
            ChangeTag::Equal => {}
        }
    }
    (added, removed, false)
}

fn fingerprint(title: &str, description: &str, content: &str) -> String {
    let mut hasher = Sha256::new();
    for value in [title, description, content] {
        let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
        hasher.update((normalized.len() as u64).to_be_bytes());
        hasher.update(normalized.as_bytes());
    }
    let digest = hasher.finalize();
    let hex = b"0123456789abcdef";
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push(hex[(byte >> 4) as usize] as char);
        output.push(hex[(byte & 0x0f) as usize] as char);
    }
    output
}

fn canonicalize_url(raw_url: &str) -> Result<String, VrtCError> {
    let mut url = Url::parse(raw_url).map_err(|_| {
        delta_error(
            "INVALID_DELTA_URL",
            "Snapshot URL must be an absolute HTTP or HTTPS URL",
            ErrorCategory::Validation,
            false,
        )
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(delta_error(
            "UNSUPPORTED_DELTA_URL",
            "Only credential-free absolute HTTP and HTTPS URLs can be versioned",
            ErrorCategory::PolicyDenied,
            false,
        ));
    }
    url.set_fragment(None);
    Ok(url.to_string())
}

fn delta_error(code: &str, message: &str, category: ErrorCategory, retryable: bool) -> VrtCError {
    VrtCError::new(code, message, category, retryable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn observed(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn records_created_unchanged_and_modified_versions() {
        let engine = DeltaEngine::new(DeltaConfig::default()).unwrap();
        let created = engine
            .record_snapshot(
                "https://example.org/page#section",
                "Page",
                "Description",
                "alpha\nbeta\n",
                observed(1),
            )
            .unwrap();
        assert_eq!(created.kind, ChangeKind::Created);
        assert_eq!(created.version, 1);

        let unchanged = engine
            .record_snapshot(
                "https://example.org/page",
                "Page",
                "Description",
                "alpha\nbeta\n",
                observed(2),
            )
            .unwrap();
        assert_eq!(unchanged.kind, ChangeKind::Unchanged);
        assert_eq!(unchanged.version, 1);

        let modified = engine
            .record_snapshot(
                "https://example.org/page",
                "Updated Page",
                "Description",
                "alpha\ngamma\ndelta\n",
                observed(3),
            )
            .unwrap();
        assert_eq!(modified.kind, ChangeKind::Modified);
        assert_eq!(modified.version, 2);
        assert!(modified.metadata_changed);
        assert_eq!(modified.added_lines, 2);
        assert_eq!(modified.removed_lines, 1);
        assert_eq!(engine.stats().unwrap().retained_versions, 2);
    }

    #[test]
    fn bounds_history_and_snapshot_size() {
        let engine = DeltaEngine::new(DeltaConfig {
            max_versions_per_url: 2,
            max_snapshot_bytes: 8,
            ..DeltaConfig::default()
        })
        .unwrap();
        for (second, content) in [(1, "one"), (2, "two"), (3, "three")] {
            engine
                .record_snapshot("https://example.org/", "T", "", content, observed(second))
                .unwrap();
        }
        let history = engine.history("https://example.org/", 2).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].version, 2);
        assert_eq!(history[1].version, 3);
        assert!(engine
            .record_snapshot(
                "https://example.org/large",
                "too long",
                "",
                "content",
                observed(4),
            )
            .is_err());
    }

    #[test]
    fn rejects_unsafe_or_invalid_snapshot_urls() {
        let engine = DeltaEngine::new(DeltaConfig::default()).unwrap();
        assert!(engine
            .record_snapshot("file:///etc/passwd", "x", "", "x", observed(1))
            .is_err());
        assert!(engine
            .record_snapshot("https://user:pass@example.org/", "x", "", "x", observed(1))
            .is_err());
    }
}
