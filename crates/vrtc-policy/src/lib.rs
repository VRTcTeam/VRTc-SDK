//! Native policy decisions for URL retrieval and crawl scope.
//!
//! DNS resolution, address pinning, redirect revalidation, and robots.txt
//! parsing remain responsibilities of the HTTP Fetcher and Crawler. This
//! engine provides a shared, explicit policy gate before those operations.

use std::net::IpAddr;

use url::Url;
use vrtc_core::{ErrorCategory, VrtCError};

#[derive(Debug, Clone)]
pub struct PolicyConfig {
    pub max_url_bytes: usize,
    pub max_crawl_depth: u8,
    pub allow_http: bool,
    pub allow_https: bool,
    pub allowed_ports: Vec<u16>,
    pub same_origin_default: bool,
    pub require_robots_for_crawl: bool,
    pub block_non_public_addresses: bool,
    pub allowed_hosts: Vec<String>,
    pub denied_hosts: Vec<String>,
    pub allowed_origins: Vec<String>,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            max_url_bytes: 8 * 1024,
            max_crawl_depth: 5,
            allow_http: true,
            allow_https: true,
            allowed_ports: vec![80, 443],
            same_origin_default: true,
            require_robots_for_crawl: true,
            block_non_public_addresses: true,
            allowed_hosts: Vec::new(),
            denied_hosts: Vec::new(),
            allowed_origins: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRequest {
    pub url: String,
    pub origin_url: Option<String>,
    pub is_crawl: bool,
    pub crawl_depth: u8,
    /// Must be set only after the Crawler has evaluated the applicable robots rule.
    pub robots_allowed: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyOutcome {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyReason {
    Allowed,
    InvalidUrl,
    UnsupportedScheme,
    PortNotAllowed,
    CredentialsForbidden,
    HostDenied,
    HostNotAllowlisted,
    NonPublicAddress,
    CrossOriginDenied,
    CrawlDepthExceeded,
    RobotsNotApproved,
    InvalidOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDecision {
    pub outcome: PolicyOutcome,
    pub reason: PolicyReason,
    pub normalized_url: Option<String>,
    pub requires_robots_evaluation: bool,
}

pub struct PolicyEngine {
    config: PolicyConfig,
}

impl PolicyEngine {
    pub fn new(mut config: PolicyConfig) -> Result<Self, VrtCError> {
        if config.max_url_bytes == 0
            || config.max_crawl_depth == 0
            || (!config.allow_http && !config.allow_https)
            || config.allowed_ports.is_empty()
            || config.allowed_ports.contains(&0)
            || config
                .allowed_hosts
                .iter()
                .any(|host| !valid_host_rule(host))
            || config
                .denied_hosts
                .iter()
                .any(|host| !valid_host_rule(host))
            || config.allowed_origins.iter().any(|origin| {
                Url::parse(origin).map_or(true, |url| {
                    !matches!(url.scheme(), "http" | "https")
                        || !url.username().is_empty()
                        || url.password().is_some()
                })
            })
        {
            return Err(policy_error(
                "INVALID_POLICY_CONFIG",
                "Policy limits, scheme settings, host rules, or allowed origins are invalid",
            ));
        }
        config.allowed_hosts = config
            .allowed_hosts
            .iter()
            .map(|host| normalize_host_rule(host))
            .collect();
        config.denied_hosts = config
            .denied_hosts
            .iter()
            .map(|host| normalize_host_rule(host))
            .collect();
        config.allowed_origins = config
            .allowed_origins
            .iter()
            .filter_map(|origin| Url::parse(origin).ok())
            .map(|url| url.origin().ascii_serialization())
            .collect();
        Ok(Self { config })
    }

    pub fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyDecision, VrtCError> {
        if request.url.len() > self.config.max_url_bytes {
            return Ok(denied(PolicyReason::InvalidUrl));
        }
        let mut target = match Url::parse(&request.url) {
            Ok(url) => url,
            Err(_) => return Ok(denied(PolicyReason::InvalidUrl)),
        };
        if !target.username().is_empty() || target.password().is_some() {
            return Ok(denied(PolicyReason::CredentialsForbidden));
        }
        let scheme_allowed = match target.scheme() {
            "http" => self.config.allow_http,
            "https" => self.config.allow_https,
            _ => false,
        };
        if !scheme_allowed {
            return Ok(denied(PolicyReason::UnsupportedScheme));
        }
        if !self
            .config
            .allowed_ports
            .contains(&target.port_or_known_default().unwrap_or(0))
        {
            return Ok(denied(PolicyReason::PortNotAllowed));
        }
        let Some(host) = target.host_str().map(normalize_host) else {
            return Ok(denied(PolicyReason::InvalidUrl));
        };
        if self
            .config
            .denied_hosts
            .iter()
            .any(|rule| host_matches(&host, rule))
        {
            return Ok(denied(PolicyReason::HostDenied));
        }
        if !self.config.allowed_hosts.is_empty()
            && !self
                .config
                .allowed_hosts
                .iter()
                .any(|rule| host_matches(&host, rule))
        {
            return Ok(denied(PolicyReason::HostNotAllowlisted));
        }
        if self.config.block_non_public_addresses && is_non_public_host(&host) {
            return Ok(denied(PolicyReason::NonPublicAddress));
        }
        if request.is_crawl && request.crawl_depth > self.config.max_crawl_depth {
            return Ok(denied(PolicyReason::CrawlDepthExceeded));
        }

        if self.config.same_origin_default {
            if let Some(origin) = request.origin_url.as_deref() {
                let Ok(origin) = Url::parse(origin) else {
                    return Ok(denied(PolicyReason::InvalidOrigin));
                };
                if !matches!(origin.scheme(), "http" | "https")
                    || !origin.username().is_empty()
                    || origin.password().is_some()
                {
                    return Ok(denied(PolicyReason::InvalidOrigin));
                }
                let same_origin = target.origin() == origin.origin();
                let explicitly_allowed = self
                    .config
                    .allowed_origins
                    .iter()
                    .any(|allowed| allowed == &target.origin().ascii_serialization());
                if !same_origin && !explicitly_allowed {
                    return Ok(denied(PolicyReason::CrossOriginDenied));
                }
            }
        }

        if request.is_crawl
            && self.config.require_robots_for_crawl
            && request.robots_allowed != Some(true)
        {
            let mut decision = denied(PolicyReason::RobotsNotApproved);
            decision.requires_robots_evaluation = request.robots_allowed.is_none();
            return Ok(decision);
        }

        target.set_fragment(None);
        Ok(PolicyDecision {
            outcome: PolicyOutcome::Allow,
            reason: PolicyReason::Allowed,
            normalized_url: Some(target.to_string()),
            requires_robots_evaluation: false,
        })
    }
}

fn denied(reason: PolicyReason) -> PolicyDecision {
    PolicyDecision {
        outcome: PolicyOutcome::Deny,
        reason,
        normalized_url: None,
        requires_robots_evaluation: false,
    }
}

fn valid_host_rule(host: &str) -> bool {
    let host = host.trim().trim_start_matches("*.");
    if host.is_empty() || host.len() > 253 || host.chars().any(char::is_control) {
        return false;
    }
    Url::parse(&format!("http://{host}/")).is_ok_and(|url| {
        url.host_str()
            .is_some_and(|parsed| normalize_host(parsed) == normalize_host(host))
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn normalize_host_rule(host: &str) -> String {
    normalize_host(host.trim().trim_start_matches("*."))
}

fn normalize_host(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

fn host_matches(host: &str, rule: &str) -> bool {
    host == rule
        || host
            .strip_suffix(rule)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn is_non_public_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return true;
    }
    let Ok(address) = host.parse::<IpAddr>() else {
        return false;
    };
    match address {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_documentation()
                || octets[0] == 0
                || octets[0] >= 240
                || (octets[0] == 100 && (octets[1] & 0xc0) == 64)
                || (octets[0] == 192 && octets[1] == 0)
                || (octets[0] == 198 && matches!(octets[1], 18 | 19))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            let global_unicast = segments[0] & 0xe000 == 0x2000;
            !global_unicast
                || ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| is_non_public_host(&mapped.to_string()))
        }
    }
}

fn policy_error(code: &str, message: &str) -> VrtCError {
    VrtCError::new(code, message, ErrorCategory::Validation, false)
        .for_component("engine.vrtc.policy")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(url: &str) -> PolicyRequest {
        PolicyRequest {
            url: url.to_owned(),
            origin_url: None,
            is_crawl: false,
            crawl_depth: 0,
            robots_allowed: None,
        }
    }

    #[test]
    fn allows_public_https_and_removes_fragment() {
        let engine = PolicyEngine::new(PolicyConfig::default()).unwrap();
        let decision = engine
            .evaluate(&request("https://example.org/page#section"))
            .unwrap();
        assert_eq!(decision.outcome, PolicyOutcome::Allow);
        assert_eq!(
            decision.normalized_url.as_deref(),
            Some("https://example.org/page")
        );
    }

    #[test]
    fn blocks_credentials_private_addresses_and_unsupported_schemes() {
        let engine = PolicyEngine::new(PolicyConfig::default()).unwrap();
        assert_eq!(
            engine
                .evaluate(&request("https://user:pass@example.org"))
                .unwrap()
                .reason,
            PolicyReason::CredentialsForbidden
        );
        assert_eq!(
            engine
                .evaluate(&request("http://127.0.0.1"))
                .unwrap()
                .reason,
            PolicyReason::NonPublicAddress
        );
        assert_eq!(
            engine
                .evaluate(&request("file:///etc/passwd"))
                .unwrap()
                .reason,
            PolicyReason::UnsupportedScheme
        );
    }

    #[test]
    fn enforces_same_origin_and_host_rules() {
        let engine = PolicyEngine::new(PolicyConfig {
            denied_hosts: vec!["blocked.example".to_owned()],
            ..PolicyConfig::default()
        })
        .unwrap();
        let mut cross_origin = request("https://other.example.org/page");
        cross_origin.origin_url = Some("https://example.org".to_owned());
        assert_eq!(
            engine.evaluate(&cross_origin).unwrap().reason,
            PolicyReason::CrossOriginDenied
        );
        assert_eq!(
            engine
                .evaluate(&request("https://blocked.example/page"))
                .unwrap()
                .reason,
            PolicyReason::HostDenied
        );
    }

    #[test]
    fn rejects_non_standard_ports_and_invalid_origins() {
        let engine = PolicyEngine::new(PolicyConfig::default()).unwrap();
        assert_eq!(
            engine
                .evaluate(&request("https://example.org:8443/path"))
                .unwrap()
                .reason,
            PolicyReason::PortNotAllowed
        );
        let mut invalid_origin = request("https://example.org/path");
        invalid_origin.origin_url = Some("file:///tmp".to_owned());
        assert_eq!(
            engine.evaluate(&invalid_origin).unwrap().reason,
            PolicyReason::InvalidOrigin
        );
    }

    #[test]
    fn crawl_requires_depth_and_robots_approval() {
        let engine = PolicyEngine::new(PolicyConfig::default()).unwrap();
        let mut crawl = request("https://example.org/page");
        crawl.is_crawl = true;
        crawl.crawl_depth = 1;
        let pending = engine.evaluate(&crawl).unwrap();
        assert_eq!(pending.reason, PolicyReason::RobotsNotApproved);
        assert!(pending.requires_robots_evaluation);
        crawl.robots_allowed = Some(true);
        assert_eq!(
            engine.evaluate(&crawl).unwrap().outcome,
            PolicyOutcome::Allow
        );
        crawl.crawl_depth = 6;
        assert_eq!(
            engine.evaluate(&crawl).unwrap().reason,
            PolicyReason::CrawlDepthExceeded
        );
    }
}
