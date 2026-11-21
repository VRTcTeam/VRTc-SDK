//! Native, policy-aware VRTc crawler orchestration.
//!
//! Composes the URL Frontier, public-address HTTP Fetcher, robots.txt policy,
//! HTML Extraction, and Document Index. This crate owns the crawl lifecycle;
//! each underlying engine remains independently usable.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use robotstxt::DefaultMatcher;
use url::Url;
use vrtc_core::{ErrorCategory, VrtCError};
use vrtc_extract::ExtractionEngine;
use vrtc_fetcher::FetchEngine;
use vrtc_frontier::{CrawlRequest, EnqueueOutcome, FrontierEngine, FrontierStats, RetryOutcome};
use vrtc_delta::{ChangeKind, DeltaEngine};
use vrtc_index::{IndexDocument, IndexEngine};

#[derive(Debug, Clone)]
pub struct CrawlerConfig {
    pub max_pages_per_run: usize,
    pub max_seeds_per_run: usize,
    pub max_links_per_page: usize,
    pub max_depth: usize,
    pub same_origin_only: bool,
    pub user_agent_product_token: String,
    pub robots_cache_ttl: Duration,
    pub robots_failure_cache_ttl: Duration,
    pub max_robots_bytes: usize,
    pub idle_poll_interval: Duration,
    pub max_idle_wait: Duration,
}

impl Default for CrawlerConfig {
    fn default() -> Self {
        Self {
            max_pages_per_run: 100,
            max_seeds_per_run: 100,
            max_links_per_page: 100,
            max_depth: 2,
            same_origin_only: true,
            user_agent_product_token: "VRTc-Fetcher".to_owned(),
            robots_cache_ttl: Duration::from_secs(24 * 60 * 60),
            robots_failure_cache_ttl: Duration::from_secs(5 * 60),
            max_robots_bytes: 512 * 1024,
            idle_poll_interval: Duration::from_millis(100),
            max_idle_wait: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RobotsDecision {
    AllowAll,
    DenyAll,
}

#[derive(Debug, Clone)]
enum CachedRobots {
    Rules {
        body: String,
        expires_at: Instant,
    },
    Decision {
        value: RobotsDecision,
        expires_at: Instant,
    },
}

impl CachedRobots {
    fn permits(&self, user_agent: &str, url: &str) -> bool {
        match self {
            Self::Rules { body, .. } => {
                let mut matcher = DefaultMatcher::default();
                matcher.one_agent_allowed_by_robots(body, user_agent, url)
            }
            Self::Decision {
                value: RobotsDecision::AllowAll,
                ..
            } => true,
            Self::Decision {
                value: RobotsDecision::DenyAll,
                ..
            } => false,
        }
    }

    fn is_fresh(&self, now: Instant) -> bool {
        match self {
            Self::Rules { expires_at, .. } | Self::Decision { expires_at, .. } => *expires_at > now,
        }
    }
}

#[derive(Default)]
struct RobotsCache {
    entries: HashMap<String, CachedRobots>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CrawlRunReport {
    pub seeds_queued: usize,
    pub seed_duplicates: usize,
    pub pages_attempted: usize,
    pub pages_fetched: usize,
    pub documents_indexed: usize,
    pub robots_denied: usize,
    pub policy_denied: usize,
    pub non_html_responses: usize,
    pub non_success_statuses: usize,
    pub extraction_failures: usize,
    pub fetch_failures: usize,
    pub retry_exhausted: usize,
    pub links_queued: usize,
    pub duplicate_links: usize,
    pub out_of_scope_links: usize,
    pub links_rejected: usize,
    pub documents_created: usize,
    pub documents_modified: usize,
    pub documents_unchanged: usize,
    pub delta_errors: usize,
    pub frontier: Option<FrontierStats>,
}

pub struct CrawlerEngine {
    config: CrawlerConfig,
    frontier: Arc<FrontierEngine>,
    fetcher: Arc<FetchEngine>,
    extraction: Arc<ExtractionEngine>,
    index: Arc<IndexEngine>,
    delta: Arc<DeltaEngine>,
    robots: Mutex<RobotsCache>,
}

impl CrawlerEngine {
    pub fn new(
        config: CrawlerConfig,
        frontier: Arc<FrontierEngine>,
        fetcher: Arc<FetchEngine>,
        extraction: Arc<ExtractionEngine>,
        index: Arc<IndexEngine>,
        delta: Arc<DeltaEngine>,
    ) -> Result<Self, VrtCError> {
        if config.max_pages_per_run == 0
            || config.max_seeds_per_run == 0
            || config.max_links_per_page == 0
            || config.user_agent_product_token.trim().is_empty()
            || config.robots_cache_ttl.is_zero()
            || config.robots_cache_ttl > Duration::from_secs(24 * 60 * 60)
            || config.robots_failure_cache_ttl.is_zero()
            || config.max_robots_bytes < 512 * 1024
            || config.idle_poll_interval.is_zero()
            || config.max_idle_wait.is_zero()
        {
            return Err(crawler_error(
                "INVALID_CRAWLER_CONFIG",
                "Crawler limits, user-agent token, robots cache, and idle wait settings must be valid",
                ErrorCategory::Validation,
                false,
            ));
        }

        Ok(Self {
            config,
            frontier,
            fetcher,
            extraction,
            index,
            delta,
            robots: Mutex::new(RobotsCache::default()),
        })
    }

    /// Enqueue seed URLs and process a bounded crawl run.
    ///
    /// The run is sequential by design. Frontier host politeness, robots rules,
    /// and the HTTP Fetcher's public-address checks apply to every page.
    pub async fn crawl(&self, seeds: &[String]) -> Result<CrawlRunReport, VrtCError> {
        if seeds.is_empty() || seeds.len() > self.config.max_seeds_per_run {
            return Err(crawler_error(
                "INVALID_CRAWL_SEED_COUNT",
                "Seed count must be within the configured run limit",
                ErrorCategory::Validation,
                false,
            ));
        }

        let mut report = CrawlRunReport::default();
        for seed in seeds {
            match self.frontier.enqueue(CrawlRequest::new(seed.clone()))? {
                EnqueueOutcome::Queued => report.seeds_queued += 1,
                EnqueueOutcome::Duplicate => report.seed_duplicates += 1,
            }
        }

        let mut idle_since = None;
        while report.pages_attempted < self.config.max_pages_per_run {
            let now = SystemTime::now();
            let Some(lease) = self.frontier.lease_next(now)? else {
                let stats = self.frontier.stats()?;
                if stats.pending == 0 {
                    break;
                }
                let idle_start = idle_since.get_or_insert_with(Instant::now);
                if idle_start.elapsed() >= self.config.max_idle_wait {
                    break;
                }
                tokio::time::sleep(self.config.idle_poll_interval).await;
                continue;
            };
            idle_since = None;
            report.pages_attempted += 1;

            if !self.robots_allowed(&lease.request.url).await {
                self.frontier.complete(&lease.id)?;
                report.robots_denied += 1;
                continue;
            }

            let response = match self.fetcher.fetch(&lease.request.url).await {
                Ok(response) => response,
                Err(error) => {
                    if matches!(
                        error.category,
                        ErrorCategory::PolicyDenied | ErrorCategory::Validation
                    ) {
                        self.frontier.complete(&lease.id)?;
                        report.policy_denied += 1;
                    } else {
                        report.fetch_failures += 1;
                        self.retry_lease(&lease.id, Duration::from_secs(2), &mut report)?;
                    }
                    continue;
                }
            };

            if response.status == 429 || response.status >= 500 {
                report.fetch_failures += 1;
                self.retry_lease(&lease.id, Duration::from_secs(30), &mut report)?;
                continue;
            }
            if !(200..300).contains(&response.status) {
                self.frontier.complete(&lease.id)?;
                report.non_success_statuses += 1;
                continue;
            }

            let content_type = response
                .content_type
                .as_deref()
                .unwrap_or_default()
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if content_type != "text/html" && content_type != "application/xhtml+xml" {
                self.frontier.complete(&lease.id)?;
                report.non_html_responses += 1;
                continue;
            }

            report.pages_fetched += 1;
            let html = String::from_utf8_lossy(&response.body);
            let extracted = match self.extraction.extract(&html, Some(&response.final_url)) {
                Ok(document) => document,
                Err(_) => {
                    self.frontier.complete(&lease.id)?;
                    report.extraction_failures += 1;
                    continue;
                }
            };

            let title = extracted
                .title
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| response.final_url.clone());
            let document = IndexDocument {
                id: response.final_url.clone(),
                url: extracted
                    .canonical_url
                    .clone()
                    .unwrap_or_else(|| response.final_url.clone()),
                title,
                description: extracted.description.clone().unwrap_or_default(),
                body: extracted.text.clone(),
                language: extracted.language.clone().unwrap_or_default(),
                published_at: extracted.published_at.clone().unwrap_or_default(),
                source: "vrtc-native-crawler".to_owned(),
            };

            if self
                .index
                .index_batch(std::slice::from_ref(&document))
                .is_err()
            {
                self.retry_lease(&lease.id, Duration::from_secs(2), &mut report)?;
                continue;
            }

            report.documents_indexed += 1;
            match self.delta.record_snapshot(
                &document.url,
                &document.title,
                &document.description,
                &document.body,
                SystemTime::now(),
            ) {
                Ok(record) => match record.kind {
                    ChangeKind::Created => report.documents_created += 1,
                    ChangeKind::Modified => report.documents_modified += 1,
                    ChangeKind::Unchanged => report.documents_unchanged += 1,
                },
                Err(_) => report.delta_errors += 1,
            }
            let source_host = origin_of(&response.final_url);
            for link in extracted.links.iter().take(self.config.max_links_per_page) {
                if lease.request.depth >= self.config.max_depth {
                    break;
                }
                if self.config.same_origin_only && origin_of(&link.url) != source_host {
                    report.out_of_scope_links += 1;
                    continue;
                }
                let mut request = CrawlRequest::new(link.url.clone());
                request.depth = lease.request.depth + 1;
                request.discovered_from = Some(response.final_url.clone());
                match self.frontier.enqueue(request) {
                    Ok(EnqueueOutcome::Queued) => report.links_queued += 1,
                    Ok(EnqueueOutcome::Duplicate) => report.duplicate_links += 1,
                    Err(_) => report.links_rejected += 1,
                }
            }
            self.frontier.complete(&lease.id)?;
        }

        report.frontier = Some(self.frontier.stats()?);
        Ok(report)
    }

    async fn robots_allowed(&self, raw_url: &str) -> bool {
        let Ok(url) = Url::parse(raw_url) else {
            return false;
        };
        let origin = url.origin().ascii_serialization();
        let now = Instant::now();
        if let Ok(cache) = self.robots.lock() {
            if let Some(entry) = cache.entries.get(&origin) {
                if entry.is_fresh(now) {
                    return entry.permits(&self.config.user_agent_product_token, raw_url);
                }
            }
        }

        let robots_url = format!("{origin}/robots.txt");
        let decision = match self.fetcher.fetch(&robots_url).await {
            Ok(response) if (200..300).contains(&response.status) => {
                let content_type = response
                    .content_type
                    .as_deref()
                    .unwrap_or_default()
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim();
                if response.body.len() > self.config.max_robots_bytes
                    || !content_type.eq_ignore_ascii_case("text/plain")
                {
                    CachedRobots::Decision {
                        value: RobotsDecision::DenyAll,
                        expires_at: Instant::now() + self.config.robots_failure_cache_ttl,
                    }
                } else if let Ok(body) = String::from_utf8(response.body) {
                    CachedRobots::Rules {
                        body,
                        expires_at: Instant::now() + self.config.robots_cache_ttl,
                    }
                } else {
                    CachedRobots::Decision {
                        value: RobotsDecision::DenyAll,
                        expires_at: Instant::now() + self.config.robots_failure_cache_ttl,
                    }
                }
            }
            Ok(response) if response.status == 404 || response.status == 410 => {
                CachedRobots::Decision {
                    value: RobotsDecision::AllowAll,
                    expires_at: Instant::now() + self.config.robots_cache_ttl,
                }
            }
            _ => CachedRobots::Decision {
                value: RobotsDecision::DenyAll,
                expires_at: Instant::now() + self.config.robots_failure_cache_ttl,
            },
        };
        let allowed = decision.permits(&self.config.user_agent_product_token, raw_url);
        if let Ok(mut cache) = self.robots.lock() {
            cache.entries.insert(origin, decision);
        }
        allowed
    }

    fn retry_lease(
        &self,
        lease_id: &str,
        delay: Duration,
        report: &mut CrawlRunReport,
    ) -> Result<(), VrtCError> {
        match self.frontier.retry(lease_id, SystemTime::now(), delay)? {
            RetryOutcome::Requeued => {}
            RetryOutcome::Exhausted => report.retry_exhausted += 1,
        }
        Ok(())
    }
}

fn origin_of(raw_url: &str) -> String {
    Url::parse(raw_url)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_default()
}

fn crawler_error(code: &str, message: &str, category: ErrorCategory, retryable: bool) -> VrtCError {
    VrtCError::new(code, message, category, retryable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robots_policy_uses_user_agent_specific_rules() {
        let body = "User-agent: VRTc-Fetcher\nDisallow: /private\nAllow: /private/public\n";
        let rules = CachedRobots::Rules {
            body: body.to_owned(),
            expires_at: Instant::now() + Duration::from_secs(60),
        };
        assert!(!rules.permits("VRTc-Fetcher", "https://example.org/private/secret"));
        assert!(rules.permits("VRTc-Fetcher", "https://example.org/private/public/page"));
    }

    #[test]
    fn same_origin_scope_distinguishes_scheme_and_port() {
        assert_eq!(
            origin_of("https://example.org:8443/page"),
            origin_of("https://example.org:8443/other")
        );
        assert_ne!(
            origin_of("https://example.org/page"),
            origin_of("http://example.org/page")
        );
        assert_ne!(
            origin_of("https://example.org:8443/page"),
            origin_of("https://example.org:9443/page")
        );
    }

    #[test]
    fn robots_policy_allows_unavailable_not_found_responses() {
        let allow = CachedRobots::Decision {
            value: RobotsDecision::AllowAll,
            expires_at: Instant::now() + Duration::from_secs(60),
        };
        assert!(allow.permits("VRTc-Fetcher", "https://example.org/page"));
    }

    #[test]
    fn robots_policy_fails_closed_for_unreachable_origins() {
        let deny = CachedRobots::Decision {
            value: RobotsDecision::DenyAll,
            expires_at: Instant::now() + Duration::from_secs(60),
        };
        assert!(!deny.permits("VRTc-Fetcher", "https://example.org/page"));
    }
}
