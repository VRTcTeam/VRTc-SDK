//! Native VRTc URL frontier and scheduler.
//!
//! The frontier owns URL identity, deduplication, priority, depth limits,
//! per-host politeness, leases, retries, and recovery. It does not fetch pages.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime};

use url::Url;
use vrtc_core::{ErrorCategory, VrtCError};

#[derive(Debug, Clone)]
pub struct FrontierConfig {
    pub max_pending: usize,
    pub max_in_flight: usize,
    pub max_depth: usize,
    pub max_attempts: u32,
    pub lease_duration: Duration,
    pub per_host_delay: Duration,
}

impl Default for FrontierConfig {
    fn default() -> Self {
        Self {
            max_pending: 50_000,
            max_in_flight: 128,
            max_depth: 10,
            max_attempts: 3,
            lease_duration: Duration::from_secs(60),
            per_host_delay: Duration::from_millis(1_000),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrawlRequest {
    pub url: String,
    pub depth: usize,
    pub priority: i32,
    pub discovered_from: Option<String>,
    pub not_before: SystemTime,
    pub attempts: u32,
}

impl CrawlRequest {
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            depth: 0,
            priority: 0,
            discovered_from: None,
            not_before: SystemTime::now(),
            attempts: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrawlLease {
    pub id: String,
    pub request: CrawlRequest,
    pub leased_until: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueOutcome {
    Queued,
    Duplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryOutcome {
    Requeued,
    Exhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontierStats {
    pub pending: usize,
    pub in_flight: usize,
    pub completed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone)]
struct PendingEntry {
    canonical_url: String,
    host: String,
    request: CrawlRequest,
    sequence: u64,
}

#[derive(Debug, Clone)]
struct InFlightEntry {
    entry: PendingEntry,
    leased_until: SystemTime,
}

#[derive(Debug, Default)]
struct FrontierState {
    pending: HashMap<String, PendingEntry>,
    in_flight: HashMap<String, InFlightEntry>,
    completed: HashSet<String>,
    failed: HashMap<String, CrawlRequest>,
    host_next_allowed: HashMap<String, SystemTime>,
    next_sequence: u64,
    next_lease: u64,
}

/// Thread-safe native URL frontier.
pub struct FrontierEngine {
    config: FrontierConfig,
    state: std::sync::Mutex<FrontierState>,
}

impl FrontierEngine {
    pub fn new(config: FrontierConfig) -> Result<Self, VrtCError> {
        if config.max_pending == 0
            || config.max_in_flight == 0
            || config.max_attempts == 0
            || config.lease_duration.is_zero()
        {
            return Err(VrtCError::new(
                "INVALID_FRONTIER_CONFIG",
                "Queue, in-flight, attempt, and lease limits must be positive",
                ErrorCategory::Validation,
                false,
            ));
        }
        Ok(Self {
            config,
            state: std::sync::Mutex::new(FrontierState::default()),
        })
    }

    /// Add a URL unless it is already pending, leased, completed, or failed.
    pub fn enqueue(&self, mut request: CrawlRequest) -> Result<EnqueueOutcome, VrtCError> {
        let (canonical_url, host) = canonicalize_url(&request.url)?;
        if request.depth > self.config.max_depth {
            return Err(VrtCError::new(
                "CRAWL_DEPTH_EXCEEDED",
                "Crawl request exceeds the configured maximum depth",
                ErrorCategory::PolicyDenied,
                false,
            ));
        }
        request.url = canonical_url.clone();
        let mut state = self.lock_state()?;
        if state.pending.contains_key(&canonical_url)
            || state
                .in_flight
                .values()
                .any(|entry| entry.entry.canonical_url == canonical_url)
            || state.completed.contains(&canonical_url)
            || state.failed.contains_key(&canonical_url)
        {
            return Ok(EnqueueOutcome::Duplicate);
        }
        if state.pending.len() >= self.config.max_pending {
            return Err(VrtCError::new(
                "FRONTIER_CAPACITY_REACHED",
                "The URL frontier has reached its pending queue capacity",
                ErrorCategory::ResourceExhausted,
                true,
            ));
        }
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.saturating_add(1);
        state.pending.insert(
            canonical_url.clone(),
            PendingEntry {
                canonical_url,
                host,
                request,
                sequence,
            },
        );
        Ok(EnqueueOutcome::Queued)
    }

    /// Lease the highest-priority eligible URL while respecting host politeness.
    pub fn lease_next(&self, now: SystemTime) -> Result<Option<CrawlLease>, VrtCError> {
        let mut state = self.lock_state()?;
        recover_expired_locked(&self.config, &mut state, now);
        if state.in_flight.len() >= self.config.max_in_flight {
            return Ok(None);
        }

        let candidate = state
            .pending
            .values()
            .filter(|entry| {
                entry.request.not_before <= now
                    && state
                        .host_next_allowed
                        .get(&entry.host)
                        .map_or(true, |allowed| *allowed <= now)
            })
            .max_by(|left, right| {
                left.request
                    .priority
                    .cmp(&right.request.priority)
                    .then_with(|| right.sequence.cmp(&left.sequence))
            })
            .map(|entry| entry.canonical_url.clone());

        let Some(canonical_url) = candidate else {
            return Ok(None);
        };
        let entry = state
            .pending
            .remove(&canonical_url)
            .expect("candidate exists");
        let leased_until = now + self.config.lease_duration;
        state
            .host_next_allowed
            .insert(entry.host.clone(), now + self.config.per_host_delay);
        let id = format!("frontier-lease-{}", state.next_lease);
        state.next_lease = state.next_lease.saturating_add(1);
        let request = entry.request.clone();
        state.in_flight.insert(
            id.clone(),
            InFlightEntry {
                entry,
                leased_until,
            },
        );
        Ok(Some(CrawlLease {
            id,
            request,
            leased_until,
        }))
    }

    /// Mark a leased URL as successfully processed.
    pub fn complete(&self, lease_id: &str) -> Result<(), VrtCError> {
        let mut state = self.lock_state()?;
        let entry = state
            .in_flight
            .remove(lease_id)
            .ok_or_else(|| lease_not_found(lease_id))?;
        state.completed.insert(entry.entry.canonical_url);
        Ok(())
    }

    /// Retry a leased URL after a failure, or move it to the failed set.
    pub fn retry(
        &self,
        lease_id: &str,
        now: SystemTime,
        retry_delay: Duration,
    ) -> Result<RetryOutcome, VrtCError> {
        let mut state = self.lock_state()?;
        let mut entry = state
            .in_flight
            .remove(lease_id)
            .ok_or_else(|| lease_not_found(lease_id))?
            .entry;
        entry.request.attempts = entry.request.attempts.saturating_add(1);
        if entry.request.attempts >= self.config.max_attempts {
            state.failed.insert(entry.canonical_url, entry.request);
            return Ok(RetryOutcome::Exhausted);
        }
        entry.request.not_before = now + retry_delay;
        entry.sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.saturating_add(1);
        state.pending.insert(entry.canonical_url.clone(), entry);
        Ok(RetryOutcome::Requeued)
    }

    /// Requeue expired leases, applying the same attempt budget as normal retries.
    pub fn recover_expired(&self, now: SystemTime) -> Result<usize, VrtCError> {
        let mut state = self.lock_state()?;
        Ok(recover_expired_locked(&self.config, &mut state, now))
    }

    pub fn stats(&self) -> Result<FrontierStats, VrtCError> {
        let state = self.lock_state()?;
        Ok(FrontierStats {
            pending: state.pending.len(),
            in_flight: state.in_flight.len(),
            completed: state.completed.len(),
            failed: state.failed.len(),
        })
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, FrontierState>, VrtCError> {
        self.state.lock().map_err(|_| {
            VrtCError::new(
                "FRONTIER_LOCK_POISONED",
                "The URL frontier state is temporarily unavailable",
                ErrorCategory::Internal,
                true,
            )
        })
    }
}

fn recover_expired_locked(
    config: &FrontierConfig,
    state: &mut FrontierState,
    now: SystemTime,
) -> usize {
    let expired = state
        .in_flight
        .iter()
        .filter(|(_, entry)| entry.leased_until <= now)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    let mut recovered = 0;
    for id in expired {
        if let Some(mut leased) = state.in_flight.remove(&id) {
            leased.entry.request.attempts = leased.entry.request.attempts.saturating_add(1);
            if leased.entry.request.attempts >= config.max_attempts {
                state
                    .failed
                    .insert(leased.entry.canonical_url, leased.entry.request);
            } else {
                leased.entry.request.not_before = now;
                leased.entry.sequence = state.next_sequence;
                state.next_sequence = state.next_sequence.saturating_add(1);
                state
                    .pending
                    .insert(leased.entry.canonical_url.clone(), leased.entry);
                recovered += 1;
            }
        }
    }
    recovered
}

fn canonicalize_url(raw: &str) -> Result<(String, String), VrtCError> {
    let mut url = Url::parse(raw).map_err(|_| {
        VrtCError::new(
            "INVALID_CRAWL_URL",
            "Crawl URL must be an absolute HTTP or HTTPS URL",
            ErrorCategory::Validation,
            false,
        )
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(VrtCError::new(
            "UNSUPPORTED_CRAWL_URL",
            "Only absolute HTTP and HTTPS URLs can enter the frontier",
            ErrorCategory::PolicyDenied,
            false,
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(VrtCError::new(
            "URL_CREDENTIALS_NOT_ALLOWED",
            "URLs containing embedded credentials are not accepted",
            ErrorCategory::PolicyDenied,
            false,
        ));
    }
    url.set_fragment(None);
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    Ok((url.to_string(), host))
}

fn lease_not_found(lease_id: &str) -> VrtCError {
    VrtCError::new(
        "FRONTIER_LEASE_NOT_FOUND",
        "The requested crawl lease is not active",
        ErrorCategory::NotFound,
        false,
    )
    .for_component(lease_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_urls_are_deduplicated() {
        let frontier = FrontierEngine::new(FrontierConfig::default()).unwrap();
        assert_eq!(
            frontier
                .enqueue(CrawlRequest::new("https://example.org/page#section"))
                .unwrap(),
            EnqueueOutcome::Queued
        );
        assert_eq!(
            frontier
                .enqueue(CrawlRequest::new("https://example.org/page"))
                .unwrap(),
            EnqueueOutcome::Duplicate
        );
    }

    #[test]
    fn priority_and_host_politeness_control_leasing() {
        let frontier = FrontierEngine::new(FrontierConfig {
            per_host_delay: Duration::from_secs(5),
            ..FrontierConfig::default()
        })
        .unwrap();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let mut low = CrawlRequest::new("https://example.org/low");
        low.priority = 1;
        let mut high = CrawlRequest::new("https://other.example/high");
        high.priority = 9;
        frontier.enqueue(low).unwrap();
        frontier.enqueue(high).unwrap();

        let first = frontier.lease_next(now).unwrap().unwrap();
        assert_eq!(first.request.url, "https://other.example/high");
        assert!(frontier.lease_next(now).unwrap().is_some());
        assert!(frontier.lease_next(now).unwrap().is_none());
    }

    #[test]
    fn expired_leases_are_recovered_and_attempts_are_bounded() {
        let frontier = FrontierEngine::new(FrontierConfig {
            lease_duration: Duration::from_secs(1),
            max_attempts: 2,
            ..FrontierConfig::default()
        })
        .unwrap();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        frontier
            .enqueue(CrawlRequest::new("https://example.org/"))
            .unwrap();
        let lease = frontier.lease_next(now).unwrap().unwrap();
        assert_eq!(
            frontier
                .recover_expired(now + Duration::from_secs(2))
                .unwrap(),
            1
        );
        let second = frontier
            .lease_next(now + Duration::from_secs(2) + Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert_eq!(
            frontier
                .retry(&second.id, now + Duration::from_secs(4), Duration::ZERO)
                .unwrap(),
            RetryOutcome::Exhausted
        );
        assert_eq!(frontier.stats().unwrap().failed, 1);
        assert_eq!(frontier.stats().unwrap().in_flight, 0);
        let _ = lease;
    }
}
