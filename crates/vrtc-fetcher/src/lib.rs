//! Native, policy-constrained HTTP fetcher for VRTc.
//!
//! Requests are limited to public HTTP(S) endpoints, DNS results are checked
//! and pinned for each hop, redirects are revalidated, and response bodies are
//! bounded. The fetcher does not crawl, render JavaScript, or bypass access
//! controls; those concerns belong to separate VRTc engines.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, SystemTime};

use reqwest::header::{ACCEPT, CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use reqwest::redirect::Policy;
use reqwest::{Client, StatusCode};
use url::Url;
use vrtc_core::{ErrorCategory, VrtCError};

#[derive(Debug, Clone)]
pub struct FetchConfig {
    pub timeout: Duration,
    pub max_body_bytes: usize,
    pub max_redirects: usize,
    pub user_agent: String,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(15),
            max_body_bytes: 5 * 1024 * 1024,
            max_redirects: 5,
            user_agent: "VRTc-Fetcher/0.1".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResponse {
    pub requested_url: String,
    pub final_url: String,
    pub status: u16,
    pub content_type: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub redirect_chain: Vec<String>,
    pub body: Vec<u8>,
    pub fetched_at: SystemTime,
}

pub struct FetchEngine {
    config: FetchConfig,
}

impl FetchEngine {
    pub fn new(config: FetchConfig) -> Result<Self, VrtCError> {
        if config.timeout.is_zero()
            || config.max_body_bytes == 0
            || config.max_redirects > 10
            || config.user_agent.trim().is_empty()
        {
            return Err(VrtCError::new(
                "INVALID_FETCH_CONFIG",
                "Timeout, body limit, redirect limit, and user agent must be valid",
                ErrorCategory::Validation,
                false,
            ));
        }
        Ok(Self { config })
    }

    /// Fetch one public HTTP(S) resource with bounded redirects and body size.
    pub async fn fetch(&self, raw_url: &str) -> Result<FetchResponse, VrtCError> {
        let requested = validate_url(raw_url)?;
        let requested_url = requested.to_string();
        let mut current = requested;
        let mut redirect_chain = Vec::new();

        for hop in 0..=self.config.max_redirects {
            let (host, address) = resolve_public_endpoint(&current).await?;
            let client = Client::builder()
                .timeout(self.config.timeout)
                .redirect(Policy::none())
                .no_proxy()
                .user_agent(&self.config.user_agent)
                .resolve(&host, address)
                .build()
                .map_err(|_| fetch_error(
                    "HTTP_CLIENT_INIT_FAILED",
                    "The HTTP client could not be initialized",
                    ErrorCategory::Internal,
                    true,
                ))?;

            let response = client
                .get(current.clone())
                .header(ACCEPT, "text/html,application/xhtml+xml,application/json,text/plain,application/pdf,*/*;q=0.5")
                .send()
                .await
                .map_err(|error| {
                    let category = if error.is_timeout() {
                        ErrorCategory::Timeout
                    } else {
                        ErrorCategory::Internal
                    };
                    fetch_error(
                        "HTTP_FETCH_FAILED",
                        "The HTTP request failed before a response was received",
                        category,
                        true,
                    )
                })?;

            if is_redirect(response.status()) {
                if hop == self.config.max_redirects {
                    return Err(fetch_error(
                        "FETCH_REDIRECT_LIMIT",
                        "The response exceeded the configured redirect limit",
                        ErrorCategory::ResourceExhausted,
                        false,
                    ));
                }
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| fetch_error(
                        "INVALID_REDIRECT",
                        "A redirect response did not contain a valid Location header",
                        ErrorCategory::Validation,
                        false,
                    ))?;
                let next = current.join(location).map_err(|_| fetch_error(
                    "INVALID_REDIRECT_URL",
                    "The redirect target is not a valid URL",
                    ErrorCategory::Validation,
                    false,
                ))?;
                if current.scheme() == "https" && next.scheme() == "http" {
                    return Err(fetch_error(
                        "HTTPS_DOWNGRADE_BLOCKED",
                        "HTTPS redirects to plain HTTP are not allowed",
                        ErrorCategory::PolicyDenied,
                        false,
                    ));
                }
                current = validate_url(next.as_str())?;
                redirect_chain.push(current.to_string());
                continue;
            }

            let content_length = response
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok());
            if content_length.is_some_and(|length| length > self.config.max_body_bytes) {
                return Err(fetch_error(
                    "FETCH_BODY_TOO_LARGE",
                    "The response declares a body larger than the configured limit",
                    ErrorCategory::ResourceExhausted,
                    false,
                ));
            }

            let status = response.status().as_u16();
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let headers = selected_headers(response.headers());
            let mut response = response;
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| fetch_error(
                "HTTP_BODY_READ_FAILED",
                "The response body could not be read completely",
                ErrorCategory::Internal,
                true,
            ))? {
                if body.len().saturating_add(chunk.len()) > self.config.max_body_bytes {
                    return Err(fetch_error(
                        "FETCH_BODY_TOO_LARGE",
                        "The response body exceeded the configured byte limit",
                        ErrorCategory::ResourceExhausted,
                        false,
                    ));
                }
                body.extend_from_slice(&chunk);
            }

            return Ok(FetchResponse {
                requested_url,
                final_url: current.to_string(),
                status,
                content_type,
                headers,
                redirect_chain,
                body,
                fetched_at: SystemTime::now(),
            });
        }

        Err(fetch_error(
            "FETCH_REDIRECT_LIMIT",
            "The response exceeded the configured redirect limit",
            ErrorCategory::ResourceExhausted,
            false,
        ))
    }
}

fn validate_url(raw_url: &str) -> Result<Url, VrtCError> {
    let url = Url::parse(raw_url).map_err(|_| fetch_error(
        "INVALID_FETCH_URL",
        "Fetch URL must be an absolute HTTP or HTTPS URL",
        ErrorCategory::Validation,
        false,
    ))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(fetch_error(
            "UNSUPPORTED_FETCH_URL",
            "Only absolute HTTP and HTTPS URLs are accepted",
            ErrorCategory::PolicyDenied,
            false,
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(fetch_error(
            "URL_CREDENTIALS_NOT_ALLOWED",
            "URLs containing embedded credentials are not accepted",
            ErrorCategory::PolicyDenied,
            false,
        ));
    }
    if !matches!(url.port_or_known_default(), Some(80 | 443)) {
        return Err(fetch_error(
            "FETCH_PORT_NOT_ALLOWED",
            "The HTTP fetcher only permits standard HTTP and HTTPS ports",
            ErrorCategory::PolicyDenied,
            false,
        ));
    }
    Ok(url)
}

async fn resolve_public_endpoint(url: &Url) -> Result<(String, SocketAddr), VrtCError> {
    let host = url.host_str().ok_or_else(|| fetch_error(
        "FETCH_HOST_MISSING",
        "Fetch URL does not contain a hostname",
        ErrorCategory::Validation,
        false,
    ))?.to_ascii_lowercase();
    let port = url.port_or_known_default().ok_or_else(|| fetch_error(
        "FETCH_PORT_MISSING",
        "Fetch URL does not contain a valid port",
        ErrorCategory::Validation,
        false,
    ))?;

    let addresses = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| fetch_error(
            "FETCH_DNS_FAILED",
            "The fetch hostname could not be resolved",
            ErrorCategory::Internal,
            true,
        ))?
        .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(fetch_error(
            "NON_PUBLIC_ADDRESS_BLOCKED",
            "The hostname resolves to an address that is not permitted for public fetching",
            ErrorCategory::PolicyDenied,
            false,
        ));
    }
    Ok((host, addresses[0]))
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(address) => {
            let octets = address.octets();
            !(address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_broadcast()
                || address.is_unspecified()
                || address.is_multicast()
                || address.is_documentation()
                || octets[0] == 0
                || octets[0] >= 240
                || (octets[0] == 100 && (octets[1] & 0xc0) == 64)
                || (octets[0] == 192 && octets[1] == 0)
                || (octets[0] == 198 && matches!(octets[1], 18 | 19)))
        }
        IpAddr::V6(address) => {
            let segments = address.segments();
            let global_unicast = segments[0] & 0xe000 == 0x2000;
            global_unicast
                && !address.is_loopback()
                && !address.is_unspecified()
                && !address.is_multicast()
                && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        }
    }
}

fn is_redirect(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

fn selected_headers(headers: &reqwest::header::HeaderMap) -> BTreeMap<String, String> {
    ["content-type", "content-language", "last-modified", "etag", "cache-control"]
        .into_iter()
        .filter_map(|name| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(|value| (name.to_owned(), value.to_owned()))
        })
        .collect()
}

fn fetch_error(
    code: &str,
    message: &str,
    category: ErrorCategory,
    retryable: bool,
) -> VrtCError {
    VrtCError::new(code, message, category, retryable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_schemes_and_embedded_credentials() {
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("https://user:secret@example.org/").is_err());
    }

    #[test]
    fn rejects_non_standard_ports() {
        assert!(validate_url("https://example.org:8443/").is_err());
        assert!(validate_url("http://example.org/").is_ok());
    }

    #[test]
    fn rejects_non_public_ip_ranges() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "169.254.1.1",
            "100.64.0.1",
            "198.18.0.1",
        ] {
            assert!(!is_public_ip(address.parse().unwrap()), "{address}");
        }
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
    }
}
