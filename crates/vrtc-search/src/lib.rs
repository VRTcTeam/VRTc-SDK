//! Provider-agnostic search orchestration for the VRTc SDK.
//!
//! The engine coordinates configured providers; it does not claim to own a
//! public-web index. Provider failures are isolated and returned alongside
//! successful results.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use vrtc_core::{
    ErrorCategory, ProviderSearchError, SearchRequest, SearchResponse, SearchResult, VrtCError,
};

/// Configuration for bounded provider execution.
#[derive(Debug, Clone)]
pub struct SearchConfig {
    pub max_concurrency: usize,
    pub provider_timeout: Duration,
    pub default_limit: usize,
    pub maximum_limit: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 4,
            provider_timeout: Duration::from_secs(10),
            default_limit: 10,
            maximum_limit: 100,
        }
    }
}

/// Adapter contract implemented by local or remote search providers.
#[async_trait]
pub trait SearchProvider: Send + Sync {
    fn id(&self) -> &str;

    async fn search(&self, request: SearchRequest) -> Result<Vec<SearchResult>, VrtCError>;
}

/// VRTc-owned search coordinator.
pub struct SearchEngine {
    config: SearchConfig,
    providers: RwLock<Vec<Arc<dyn SearchProvider>>>,
}

impl SearchEngine {
    /// Create an engine with validated resource limits.
    pub fn new(config: SearchConfig) -> Result<Self, VrtCError> {
        if config.max_concurrency == 0
            || config.provider_timeout.is_zero()
            || config.default_limit == 0
            || config.maximum_limit == 0
            || config.default_limit > config.maximum_limit
        {
            return Err(VrtCError::new(
                "INVALID_SEARCH_CONFIG",
                "Concurrency, timeout, and result limits must be positive and internally consistent",
                ErrorCategory::Validation,
                false,
            ));
        }

        Ok(Self {
            config,
            providers: RwLock::new(Vec::new()),
        })
    }

    /// Register a provider adapter at runtime. Registration stores an adapter
    /// handle only; it does not load native code or bypass policy.
    pub fn register_provider(&self, provider: Arc<dyn SearchProvider>) -> Result<(), VrtCError> {
        let id = provider.id();
        if id.trim().is_empty() {
            return Err(VrtCError::new(
                "INVALID_PROVIDER_ID",
                "Search provider identifier must not be empty",
                ErrorCategory::Validation,
                false,
            ));
        }

        let mut providers = self.providers.write().map_err(|_| {
            VrtCError::new(
                "SEARCH_PROVIDER_LOCK_POISONED",
                "Search provider catalog is unavailable",
                ErrorCategory::Internal,
                true,
            )
        })?;

        if providers.iter().any(|registered| registered.id() == id) {
            return Err(VrtCError::new(
                "DUPLICATE_PROVIDER_ID",
                "A search provider with this identifier is already registered",
                ErrorCategory::DuplicateId,
                false,
            )
            .for_component(id));
        }

        providers.push(provider);
        Ok(())
    }

    /// Return registered provider identifiers in deterministic order.
    #[must_use]
    pub fn provider_ids(&self) -> Vec<String> {
        let Ok(providers) = self.providers.read() else {
            return Vec::new();
        };
        let mut ids = providers
            .iter()
            .map(|provider| provider.id().to_owned())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    /// Execute a normalized query against configured providers concurrently.
    /// Individual provider errors do not cancel successful providers.
    pub async fn search(&self, mut request: SearchRequest) -> Result<SearchResponse, VrtCError> {
        request.query = normalize_query(&request.query);
        if request.query.is_empty() {
            return Err(VrtCError::new(
                "EMPTY_SEARCH_QUERY",
                "Search query must contain at least one non-whitespace character",
                ErrorCategory::Validation,
                false,
            ));
        }

        if request.limit == 0 {
            request.limit = self.config.default_limit;
        }
        if request.limit > self.config.maximum_limit {
            return Err(VrtCError::new(
                "SEARCH_LIMIT_EXCEEDED",
                "Requested result limit exceeds the configured maximum",
                ErrorCategory::Validation,
                false,
            ));
        }

        let providers = self
            .providers
            .read()
            .map_err(|_| {
                VrtCError::new(
                    "SEARCH_PROVIDER_LOCK_POISONED",
                    "Search provider catalog is unavailable",
                    ErrorCategory::Internal,
                    true,
                )
            })?
            .clone();

        if providers.is_empty() {
            return Err(VrtCError::new(
                "NO_SEARCH_PROVIDERS",
                "No search providers are registered; configure at least one local or remote adapter",
                ErrorCategory::NotRegistered,
                false,
            ));
        }

        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrency));
        let timeout = self.config.provider_timeout;
        let mut tasks = JoinSet::new();
        let mut providers_queried = Vec::with_capacity(providers.len());

        for provider in providers {
            let provider_id = provider.id().to_owned();
            providers_queried.push(provider_id.clone());
            let semaphore = Arc::clone(&semaphore);
            let request = request.clone();

            tasks.spawn(async move {
                let _permit = semaphore.acquire().await.map_err(|_| {
                    VrtCError::new(
                        "SEARCH_CAPACITY_CLOSED",
                        "Search execution capacity is unavailable",
                        ErrorCategory::ResourceExhausted,
                        true,
                    )
                })?;

                let provider_result = match tokio::time::timeout(timeout, provider.search(request))
                    .await
                {
                    Ok(result) => result.map_err(|error| error.for_component(provider_id.clone())),
                    Err(_) => Err(VrtCError::new(
                        "PROVIDER_TIMEOUT",
                        "Search provider exceeded its configured time limit",
                        ErrorCategory::Timeout,
                        true,
                    )
                    .for_component(provider_id.clone())),
                };

                provider_result.map(|results| (provider_id, results))
            });
        }

        let mut results = Vec::new();
        let mut provider_errors = Vec::new();

        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok(Ok((provider_id, mut provider_results))) => {
                    for result in &mut provider_results {
                        result.provider_id.clone_from(&provider_id);
                        if result.score.is_some_and(|score| !score.is_finite()) {
                            result.score = None;
                        }
                    }
                    results.extend(provider_results);
                }
                Ok(Err(error)) => provider_errors.push(ProviderSearchError {
                    provider_id: error
                        .component_id
                        .clone()
                        .unwrap_or_else(|| "unknown".to_owned()),
                    code: error.code,
                    message: error.message,
                    retryable: error.retryable,
                }),
                Err(_) => provider_errors.push(ProviderSearchError {
                    provider_id: "unknown".to_owned(),
                    code: "PROVIDER_TASK_FAILED".to_owned(),
                    message: "Search provider task ended unexpectedly".to_owned(),
                    retryable: true,
                }),
            }
        }

        results.retain(|result| matches_filters(result, &request));
        results.sort_by(|left, right| {
            right
                .score
                .unwrap_or(0.0)
                .total_cmp(&left.score.unwrap_or(0.0))
                .then_with(|| left.title.cmp(&right.title))
        });

        let mut seen_urls = std::collections::HashSet::new();
        results.retain(|result| seen_urls.insert(canonicalize_url(&result.url)));
        results.truncate(request.limit);

        Ok(SearchResponse {
            query: request.query,
            results,
            providers_queried,
            provider_errors,
        })
    }
}

fn normalize_query(query: &str) -> String {
    query.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn matches_filters(result: &SearchResult, request: &SearchRequest) -> bool {
    if let Some(language) = &request.language {
        if !result
            .language
            .as_ref()
            .is_some_and(|value| value.eq_ignore_ascii_case(language))
        {
            return false;
        }
    }

    if !request.domains.is_empty() {
        let host = url_host(&result.url);
        if !request.domains.iter().any(|domain| {
            let domain = domain.trim().trim_start_matches('.');
            !domain.is_empty() && (host == domain || host.ends_with(&format!(".{domain}")))
        }) {
            return false;
        }
    }

    if let Some(after) = &request.published_after {
        if !result
            .published_at
            .as_ref()
            .is_some_and(|date| date >= after)
        {
            return false;
        }
    }

    if let Some(before) = &request.published_before {
        if !result
            .published_at
            .as_ref()
            .is_some_and(|date| date <= before)
        {
            return false;
        }
    }

    true
}

fn url_host(url: &str) -> String {
    let Some((_, remainder)) = url.split_once("://") else {
        return String::new();
    };
    remainder
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .split('@')
        .next_back()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn canonicalize_url(url: &str) -> String {
    let without_fragment = url.split('#').next().unwrap_or_default();
    let Some((scheme, remainder)) = without_fragment.split_once("://") else {
        return without_fragment.trim_end_matches('/').to_owned();
    };
    let authority_end = remainder.find(['/', '?']).unwrap_or(remainder.len());
    let authority = remainder[..authority_end].to_ascii_lowercase();
    let suffix = remainder[authority_end..].trim_end_matches('/');
    format!("{}://{}{}", scheme.to_ascii_lowercase(), authority, suffix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct FixedProvider {
        provider_id: &'static str,
    }

    #[async_trait]
    impl SearchProvider for FixedProvider {
        fn id(&self) -> &str {
            self.provider_id
        }

        async fn search(&self, _request: SearchRequest) -> Result<Vec<SearchResult>, VrtCError> {
            Ok(vec![SearchResult {
                title: "VRTc documentation".to_owned(),
                url: "https://Example.com/docs/".to_owned(),
                snippet: Some("Provider-neutral search".to_owned()),
                score: Some(0.9),
                provider_id: String::new(),
                published_at: None,
                language: Some("en".to_owned()),
                metadata: BTreeMap::new(),
            }])
        }
    }

    #[tokio::test]
    async fn search_normalizes_query_and_records_provider_provenance() {
        let engine = SearchEngine::new(SearchConfig::default()).unwrap();
        engine
            .register_provider(Arc::new(FixedProvider {
                provider_id: "provider.test",
            }))
            .unwrap();

        let response = engine
            .search(SearchRequest::new("  VRTc   SDK  "))
            .await
            .unwrap();

        assert_eq!(response.query, "VRTc SDK");
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].provider_id, "provider.test");
        assert_eq!(response.providers_queried, vec!["provider.test"]);
    }

    #[tokio::test]
    async fn duplicate_provider_registration_is_rejected() {
        let engine = SearchEngine::new(SearchConfig::default()).unwrap();
        engine
            .register_provider(Arc::new(FixedProvider {
                provider_id: "provider.test",
            }))
            .unwrap();
        let error = engine
            .register_provider(Arc::new(FixedProvider {
                provider_id: "provider.test",
            }))
            .unwrap_err();

        assert_eq!(error.category, ErrorCategory::DuplicateId);
    }

    #[test]
    fn url_deduplication_ignores_host_case_trailing_slash_and_fragment() {
        assert_eq!(
            canonicalize_url("HTTPS://Example.com/docs/#section"),
            canonicalize_url("https://example.com/docs/")
        );
    }
}
