//! Geekflare Search API adapter for the VRTc Search Engine.
//!
//! Credentials are supplied by the host application and are never included in
//! provider descriptors, requests, result metadata, or formatted errors.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use vrtc_core::{
    Capability, CapabilityAvailability, ConnectionMode, ErrorCategory, LifecycleState,
    ProviderDescriptor, RegistrationSource, SearchRequest, SearchResult, VrtCError,
};
use vrtc_search::SearchProvider;

const PROVIDER_ID: &str = "provider.geekflare.search";
const SEARCH_CAPABILITY_ID: &str = "search.query";
const SEARCH_ENDPOINT: &str = "https://api.geekflare.com/search";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

/// Search collection supported by Geekflare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeekflareSearchSource {
    Web,
    News,
    Images,
}

impl GeekflareSearchSource {
    fn as_api_value(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::News => "news",
            Self::Images => "images",
        }
    }
}

/// Optional Geekflare category filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeekflareSearchCategory {
    General,
    Code,
    Pdf,
    Research,
    Linkedin,
    Wiki,
}

impl GeekflareSearchCategory {
    fn as_api_value(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Code => "code",
            Self::Pdf => "pdf",
            Self::Research => "research",
            Self::Linkedin => "linkedin",
            Self::Wiki => "wiki",
        }
    }
}

/// Static options applied to every request made by this provider instance.
///
/// Per-query result limits and domain allowlists come from the VRTc request.
#[derive(Debug, Clone)]
pub struct GeekflareSearchOptions {
    pub source: GeekflareSearchSource,
    pub location: Option<String>,
    pub city: Option<String>,
    pub time: Option<String>,
    pub category: Option<GeekflareSearchCategory>,
    pub exclude_domains: Vec<String>,
    pub scrape: bool,
    pub scrape_limit: u8,
}

impl Default for GeekflareSearchOptions {
    fn default() -> Self {
        Self {
            source: GeekflareSearchSource::Web,
            location: None,
            city: None,
            time: None,
            category: None,
            exclude_domains: Vec::new(),
            scrape: false,
            scrape_limit: 3,
        }
    }
}

/// Geekflare Search API adapter. The API key is private and redacted in Debug.
pub struct GeekflareSearchProvider {
    client: Client,
    api_key: String,
    options: GeekflareSearchOptions,
}

impl fmt::Debug for GeekflareSearchProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeekflareSearchProvider")
            .field("api_key", &"[REDACTED]")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl GeekflareSearchProvider {
    /// Create a provider using default web-search options.
    pub fn new(api_key: impl Into<String>) -> Result<Self, VrtCError> {
        Self::with_options(api_key, GeekflareSearchOptions::default())
    }

    /// Create a provider with explicit, validated API options.
    pub fn with_options(
        api_key: impl Into<String>,
        options: GeekflareSearchOptions,
    ) -> Result<Self, VrtCError> {
        let api_key = api_key.into().trim().to_owned();
        if api_key.trim().is_empty() {
            return Err(validation_error(
                "GEEKFLARE_API_KEY_MISSING",
                "Geekflare API key must be provided by the host application",
            ));
        }
        validate_options(&options)?;

        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .user_agent(concat!("VRTc-SDK/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| {
                VrtCError::new(
                    "GEEKFLARE_HTTP_CLIENT_INIT_FAILED",
                    "Could not initialize the Geekflare HTTP client",
                    ErrorCategory::Internal,
                    false,
                )
                .for_component(PROVIDER_ID)
            })?;

        Ok(Self {
            client,
            api_key,
            options,
        })
    }

    /// Build the descriptor used by the VRTc runtime registration API.
    #[must_use]
    pub fn descriptor(&self, source: RegistrationSource) -> ProviderDescriptor {
        ProviderDescriptor {
            id: PROVIDER_ID.to_owned(),
            name: "Geekflare Search API".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            contract_version: "1".to_owned(),
            capabilities: vec![Capability {
                id: SEARCH_CAPABILITY_ID.to_owned(),
                operations: vec!["search".to_owned()],
                input_contract_version: "1".to_owned(),
                output_contract_version: "1".to_owned(),
                constraints: BTreeMap::from([
                    ("sources".to_owned(), "web,news,images".to_owned()),
                    ("max_results".to_owned(), "100".to_owned()),
                    ("credential".to_owned(), "host_supplied_api_key".to_owned()),
                ]),
                availability: CapabilityAvailability::Available,
            }],
            connection_mode: ConnectionMode::RemoteApi,
            lifecycle_state: LifecycleState::Discovered,
            registration_source: source,
            metadata: BTreeMap::from([
                ("vendor".to_owned(), "Geekflare".to_owned()),
                ("api".to_owned(), "Search API".to_owned()),
                ("endpoint".to_owned(), SEARCH_ENDPOINT.to_owned()),
            ]),
        }
    }
}

#[async_trait]
impl SearchProvider for GeekflareSearchProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    async fn search(&self, request: SearchRequest) -> Result<Vec<SearchResult>, VrtCError> {
        if request.query.trim().is_empty() {
            return Err(validation_error(
                "EMPTY_SEARCH_QUERY",
                "Geekflare search query must not be empty",
            ));
        }
        if request.limit > 100 {
            return Err(validation_error(
                "GEEKFLARE_RESULT_LIMIT_EXCEEDED",
                "Geekflare supports at most 100 results per search request",
            ));
        }

        let payload = GeekflareSearchPayload {
            query: request.query,
            limit: if request.limit == 0 {
                10
            } else {
                request.limit
            },
            source: self.options.source.as_api_value(),
            location: self.options.location.as_deref(),
            city: self.options.city.as_deref(),
            time: self.options.time.as_deref(),
            category: self
                .options
                .category
                .map(GeekflareSearchCategory::as_api_value),
            include_domains: non_empty(&request.domains),
            exclude_domains: non_empty(&self.options.exclude_domains),
            format: "json",
            scrape: self.options.scrape,
            scrape_limit: self.options.scrape_limit,
        };

        let response = self
            .client
            .post(SEARCH_ENDPOINT)
            .header("x-api-key", &self.api_key)
            .json(&payload)
            .send()
            .await
            .map_err(map_transport_error)?;

        let status = response.status();
        if !status.is_success() {
            return Err(map_http_status(status));
        }

        let body: GeekflareSearchResponse = response.json().await.map_err(|_| {
            VrtCError::new(
                "GEEKFLARE_INVALID_RESPONSE",
                "Geekflare returned a response that could not be decoded",
                ErrorCategory::Internal,
                true,
            )
            .for_component(PROVIDER_ID)
        })?;

        if body
            .api_status
            .as_deref()
            .is_some_and(|value| value != "success")
        {
            return Err(VrtCError::new(
                "GEEKFLARE_API_ERROR",
                "Geekflare reported that the search request was unsuccessful",
                ErrorCategory::Internal,
                false,
            )
            .for_component(PROVIDER_ID));
        }

        let items = body.data.as_array().ok_or_else(|| {
            VrtCError::new(
                "GEEKFLARE_UNSUPPORTED_RESPONSE_MODE",
                "Geekflare returned a non-list response; this adapter expects standard search results",
                ErrorCategory::ContractIncompatible,
                false,
            )
            .for_component(PROVIDER_ID)
        })?;

        Ok(items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| normalize_result(item, index + 1))
            .collect())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeekflareSearchPayload<'a> {
    query: String,
    limit: usize,
    source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    city: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    include_domains: Option<&'a [String]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exclude_domains: Option<&'a [String]>,
    format: &'static str,
    scrape: bool,
    scrape_limit: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeekflareSearchResponse {
    #[serde(default)]
    api_status: Option<String>,
    data: Value,
}

fn normalize_result(value: &Value, fallback_position: usize) -> Option<SearchResult> {
    let title = value.get("title")?.as_str()?.trim();
    let url = value.get("url")?.as_str()?.trim();
    if title.is_empty() || url.is_empty() {
        return None;
    }

    let position = value
        .get("position")
        .and_then(Value::as_u64)
        .filter(|position| *position > 0)
        .unwrap_or(fallback_position as u64);

    let mut metadata = BTreeMap::new();
    metadata.insert("position".to_owned(), position.to_string());
    if let Some(content) = value.get("content").and_then(Value::as_str) {
        metadata.insert("content".to_owned(), content.to_owned());
    }

    if let Some(date) = value.get("date").and_then(Value::as_str) {
        metadata.insert("date".to_owned(), date.to_owned());
    }

    Some(SearchResult {
        title: title.to_owned(),
        url: url.to_owned(),
        snippet: value
            .get("snippet")
            .and_then(Value::as_str)
            .map(str::to_owned),
        score: Some(1.0 / position as f64),
        provider_id: PROVIDER_ID.to_owned(),
        published_at: None,
        language: None,
        metadata,
    })
}

fn validate_options(options: &GeekflareSearchOptions) -> Result<(), VrtCError> {
    if !(1..=10).contains(&options.scrape_limit) {
        return Err(validation_error(
            "GEEKFLARE_INVALID_SCRAPE_LIMIT",
            "Geekflare scrape limit must be between 1 and 10",
        ));
    }

    if let Some(location) = &options.location {
        if location.len() != 2 || !location.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            return Err(validation_error(
                "GEEKFLARE_INVALID_LOCATION",
                "Geekflare location must be a two-letter ISO country code",
            ));
        }
    }

    if let Some(time) = &options.time {
        let valid = matches!(time.as_str(), "h" | "d" | "w" | "m" | "y")
            || time
                .strip_prefix(['h', 'd', 'w', 'm', 'y'])
                .is_some_and(|count| {
                    !count.is_empty() && count.bytes().all(|byte| byte.is_ascii_digit())
                });
        if !valid {
            return Err(validation_error(
                "GEEKFLARE_INVALID_TIME_FILTER",
                "Geekflare time filter must use h, d, w, m, y or a count such as d7",
            ));
        }
    }

    Ok(())
}

fn non_empty(values: &[String]) -> Option<&[String]> {
    (!values.is_empty()).then_some(values)
}

fn map_transport_error(error: reqwest::Error) -> VrtCError {
    if error.is_timeout() {
        VrtCError::new(
            "GEEKFLARE_REQUEST_TIMEOUT",
            "Geekflare search request timed out",
            ErrorCategory::Timeout,
            true,
        )
        .for_component(PROVIDER_ID)
    } else {
        VrtCError::new(
            "GEEKFLARE_TRANSPORT_ERROR",
            "Could not complete the Geekflare search request",
            ErrorCategory::Internal,
            true,
        )
        .for_component(PROVIDER_ID)
    }
}

fn map_http_status(status: StatusCode) -> VrtCError {
    let (code, message, category, retryable) = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => (
            "GEEKFLARE_AUTH_FAILED",
            "Geekflare rejected the configured API credentials",
            ErrorCategory::PolicyDenied,
            false,
        ),
        StatusCode::PAYMENT_REQUIRED => (
            "GEEKFLARE_CREDITS_EXHAUSTED",
            "Geekflare account credits are exhausted",
            ErrorCategory::ResourceExhausted,
            false,
        ),
        StatusCode::TOO_MANY_REQUESTS => (
            "GEEKFLARE_RATE_LIMITED",
            "Geekflare request rate limit was reached",
            ErrorCategory::ResourceExhausted,
            true,
        ),
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => (
            "GEEKFLARE_REQUEST_TIMEOUT",
            "Geekflare search request timed out",
            ErrorCategory::Timeout,
            true,
        ),
        code if code.is_server_error() => (
            "GEEKFLARE_UPSTREAM_ERROR",
            "Geekflare search service returned a server error",
            ErrorCategory::Internal,
            true,
        ),
        _ => (
            "GEEKFLARE_REQUEST_REJECTED",
            "Geekflare rejected the search request",
            ErrorCategory::Validation,
            false,
        ),
    };

    VrtCError::new(code, message, category, retryable).for_component(PROVIDER_ID)
}

fn validation_error(code: &str, message: &str) -> VrtCError {
    VrtCError::new(code, message, ErrorCategory::Validation, false).for_component(PROVIDER_ID)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_documented_search_result_shape() {
        let value = serde_json::json!({
            "title": "VRTc documentation",
            "url": "https://example.org/docs",
            "snippet": "A provider-neutral research SDK",
            "position": 2,
            "date": "2026-11-20"
        });

        let result = normalize_result(&value, 1).unwrap();
        assert_eq!(result.title, "VRTc documentation");
        assert_eq!(result.url, "https://example.org/docs");
        assert_eq!(result.provider_id, PROVIDER_ID);
        assert_eq!(result.score, Some(0.5));
        assert_eq!(result.published_at, None);
        assert_eq!(
            result.metadata.get("date").map(String::as_str),
            Some("2026-11-20")
        );
        assert_eq!(
            result.metadata.get("position").map(String::as_str),
            Some("2")
        );
    }

    #[test]
    fn skips_results_without_a_usable_title_or_url() {
        let value = serde_json::json!({"title": "Missing URL"});
        assert!(normalize_result(&value, 1).is_none());
    }

    #[test]
    fn rejects_empty_credentials_and_invalid_options() {
        assert!(GeekflareSearchProvider::new("  ").is_err());

        let options = GeekflareSearchOptions {
            scrape_limit: 11,
            ..GeekflareSearchOptions::default()
        };
        assert!(GeekflareSearchProvider::with_options("test-key", options).is_err());
    }

    #[test]
    fn debug_output_redacts_credentials() {
        let provider = GeekflareSearchProvider::new("sensitive-test-key").unwrap();
        let rendered = format!("{provider:?}");
        assert!(!rendered.contains("sensitive-test-key"));
        assert!(rendered.contains("[REDACTED]"));
    }
}
