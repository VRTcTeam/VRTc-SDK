//! Native DNS, TLS policy, and outbound route planning primitives.
//!
//! This crate deliberately separates route planning from transport execution.
//! It never silently enables a proxy or relaxes certificate verification.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use url::Url;
use vrtc_core::{ErrorCategory, VrtCError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMinimumVersion {
    Tls12,
    Tls13,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsPolicy {
    pub minimum_version: TlsMinimumVersion,
    pub require_certificate_validation: bool,
    pub allow_invalid_certificates: bool,
}

impl Default for TlsPolicy {
    fn default() -> Self {
        Self {
            minimum_version: TlsMinimumVersion::Tls12,
            require_certificate_validation: true,
            allow_invalid_certificates: false,
        }
    }
}

impl TlsPolicy {
    pub fn validate(&self) -> Result<(), VrtCError> {
        if !self.require_certificate_validation || self.allow_invalid_certificates {
            return Err(network_error(
                "TLS_CERTIFICATE_VALIDATION_REQUIRED",
                "VRTc network policy requires certificate and hostname validation",
                ErrorCategory::PolicyDenied,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyEndpoint {
    pub id: String,
    pub url: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteMode {
    Direct,
    Proxy { proxy_id: String },
}

#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub dns_timeout: Duration,
    pub require_public_addresses: bool,
    pub tls: TlsPolicy,
    pub proxies: Vec<ProxyEndpoint>,
    pub proxy_hosts: Vec<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            dns_timeout: Duration::from_secs(5),
            require_public_addresses: true,
            tls: TlsPolicy::default(),
            proxies: Vec::new(),
            proxy_hosts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsResolution {
    pub hostname: String,
    pub port: u16,
    pub addresses: Vec<SocketAddr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkRoute {
    pub target_origin: String,
    pub mode: RouteMode,
    pub tls: TlsMinimumVersion,
}

pub struct NetworkEngine {
    config: NetworkConfig,
}

impl NetworkEngine {
    pub fn new(config: NetworkConfig) -> Result<Self, VrtCError> {
        if config.dns_timeout.is_zero() {
            return Err(network_error(
                "INVALID_DNS_TIMEOUT",
                "DNS timeout must be greater than zero",
                ErrorCategory::Validation,
            ));
        }
        config.tls.validate()?;
        let mut ids = std::collections::BTreeSet::new();
        for proxy in &config.proxies {
            if proxy.id.trim().is_empty() || !ids.insert(proxy.id.clone()) {
                return Err(network_error(
                    "INVALID_PROXY_ID",
                    "Proxy identifiers must be non-empty and unique",
                    ErrorCategory::Validation,
                ));
            }
            validate_proxy_url(&proxy.url)?;
        }
        for host in &config.proxy_hosts {
            if host.trim().is_empty()
                || host.contains('/')
                || host.contains(':')
                || host.chars().any(char::is_control)
            {
                return Err(network_error(
                    "INVALID_PROXY_HOST_RULE",
                    "Proxy host rules must contain hostnames only",
                    ErrorCategory::Validation,
                ));
            }
        }
        Ok(Self { config })
    }

    /// Resolve a hostname with a bounded timeout and reject mixed public/private answers.
    pub async fn resolve(&self, hostname: &str, port: u16) -> Result<DnsResolution, VrtCError> {
        if hostname.trim().is_empty() || port == 0 {
            return Err(network_error(
                "INVALID_DNS_QUERY",
                "DNS hostname and port are required",
                ErrorCategory::Validation,
            ));
        }
        let lookup = tokio::time::timeout(
            self.config.dns_timeout,
            tokio::net::lookup_host((hostname, port)),
        )
        .await
        .map_err(|_| {
            network_error(
                "DNS_TIMEOUT",
                "DNS lookup exceeded its time limit",
                ErrorCategory::Timeout,
            )
        })?
        .map_err(|_| {
            network_error(
                "DNS_LOOKUP_FAILED",
                "DNS lookup failed",
                ErrorCategory::Internal,
            )
        })?;
        let mut addresses: Vec<_> = lookup.collect();
        addresses.sort();
        addresses.dedup();
        if addresses.is_empty() {
            return Err(network_error(
                "DNS_NO_RECORDS",
                "DNS returned no usable addresses",
                ErrorCategory::NotFound,
            ));
        }
        if self.config.require_public_addresses
            && addresses.iter().any(|address| !is_public_ip(address.ip()))
        {
            return Err(network_error(
                "DNS_NON_PUBLIC_ADDRESS_BLOCKED",
                "DNS returned a non-public address; the entire answer set was rejected",
                ErrorCategory::PolicyDenied,
            ));
        }
        Ok(DnsResolution {
            hostname: hostname.to_ascii_lowercase(),
            port,
            addresses,
        })
    }

    /// Select a configured proxy only for explicitly matched host rules.
    /// This is a routing decision; the transport must apply it explicitly.
    pub fn plan_route(&self, raw_url: &str) -> Result<NetworkRoute, VrtCError> {
        let url = Url::parse(raw_url).map_err(|_| {
            network_error(
                "INVALID_ROUTE_URL",
                "Route target must be an absolute HTTP or HTTPS URL",
                ErrorCategory::Validation,
            )
        })?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(network_error(
                "UNSUPPORTED_ROUTE_SCHEME",
                "Only HTTP and HTTPS route targets are supported",
                ErrorCategory::PolicyDenied,
            ));
        }
        if url.scheme() == "https" {
            self.config.tls.validate()?;
        }
        let host = url
            .host_str()
            .unwrap_or_default()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        let proxy = if self
            .config
            .proxy_hosts
            .iter()
            .any(|rule| host_matches(&host, rule))
        {
            self.config.proxies.iter().find(|proxy| proxy.enabled)
        } else {
            None
        };
        Ok(NetworkRoute {
            target_origin: url.origin().ascii_serialization(),
            mode: proxy.map_or(RouteMode::Direct, |proxy| RouteMode::Proxy {
                proxy_id: proxy.id.clone(),
            }),
            tls: self.config.tls.minimum_version,
        })
    }
}

fn validate_proxy_url(raw: &str) -> Result<(), VrtCError> {
    let url = Url::parse(raw).map_err(|_| {
        network_error(
            "INVALID_PROXY_URL",
            "Proxy endpoint must be an absolute HTTP or HTTPS URL",
            ErrorCategory::Validation,
        )
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.port_or_known_default().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(network_error(
            "INVALID_PROXY_URL",
            "Proxy endpoint must use HTTP(S) and cannot contain query or fragment data",
            ErrorCategory::Validation,
        ));
    }
    Ok(())
}

fn host_matches(host: &str, rule: &str) -> bool {
    let rule = rule
        .trim()
        .trim_start_matches("*.")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    host == rule
        || host
            .strip_suffix(&rule)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let o = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_documentation()
                || o[0] == 0
                || o[0] >= 240
                || (o[0] == 100 && (o[1] & 0xc0) == 64)
                || (o[0] == 192 && o[1] == 0)
                || (o[0] == 198 && matches!(o[1], 18 | 19)))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            (s[0] & 0xe000 == 0x2000)
                && !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !(s[0] == 0x2001 && s[1] == 0x0db8)
        }
    }
}

fn network_error(code: &str, message: &str, category: ErrorCategory) -> VrtCError {
    VrtCError::new(
        code,
        message,
        category,
        category == ErrorCategory::Timeout || category == ErrorCategory::Internal,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_validation_cannot_be_disabled() {
        let policy = TlsPolicy {
            require_certificate_validation: false,
            ..TlsPolicy::default()
        };
        assert_eq!(
            policy.validate().unwrap_err().code,
            "TLS_CERTIFICATE_VALIDATION_REQUIRED"
        );
    }

    #[test]
    fn route_defaults_to_direct_and_matches_proxy_hosts_explicitly() {
        let engine = NetworkEngine::new(NetworkConfig {
            proxies: vec![ProxyEndpoint {
                id: "egress-a".into(),
                url: "http://proxy.internal:8080".into(),
                enabled: true,
            }],
            proxy_hosts: vec!["*.example.org".into()],
            ..NetworkConfig::default()
        })
        .unwrap();
        assert_eq!(
            engine
                .plan_route("https://www.example.org/path")
                .unwrap()
                .mode,
            RouteMode::Proxy {
                proxy_id: "egress-a".into()
            }
        );
        assert_eq!(
            engine.plan_route("https://other.test/").unwrap().mode,
            RouteMode::Direct
        );
    }

    #[test]
    fn rejects_invalid_proxy_configuration() {
        let result = NetworkEngine::new(NetworkConfig {
            proxies: vec![ProxyEndpoint {
                id: "bad".into(),
                url: "file:///tmp/proxy".into(),
                enabled: true,
            }],
            ..NetworkConfig::default()
        });
        assert!(result.is_err());
    }

    #[test]
    fn wildcard_host_rule_does_not_match_suffix_confusion() {
        assert!(host_matches("a.example.org", "*.example.org"));
        assert!(!host_matches("notexample.org", "*.example.org"));
    }
}
