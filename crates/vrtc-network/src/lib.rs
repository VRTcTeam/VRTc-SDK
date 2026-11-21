//! Native DNS resolution cache and transport-security policy for VRTc.
//!
//! DNS results are resolved through the host system resolver, validated as
//! public addresses, bounded, cached for a configured local TTL, and returned
//! for address pinning by the HTTP Fetcher. The cache TTL is VRTc policy; the
//! system resolver does not expose authoritative DNS record TTLs here.

use std::collections::{BTreeSet, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::ClientBuilder;
use tokio::time::timeout;
use vrtc_core::{ErrorCategory, VrtCError};

#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub max_cache_entries: usize,
    pub max_addresses_per_host: usize,
    pub cache_ttl_ms: u64,
    pub lookup_timeout_ms: u64,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            max_cache_entries: 512,
            max_addresses_per_host: 16,
            cache_ttl_ms: 30_000,
            lookup_timeout_ms: 3_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsVersion {
    Tls12,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsProfile {
    pub minimum_version: TlsVersion,
    pub certificate_validation: bool,
    pub hostname_validation: bool,
    pub https_downgrade_blocked: bool,
    pub invalid_certificates_allowed: bool,
}

impl Default for TlsProfile {
    fn default() -> Self {
        Self {
            minimum_version: TlsVersion::Tls12,
            certificate_validation: true,
            hostname_validation: true,
            https_downgrade_blocked: true,
            invalid_certificates_allowed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsResolution {
    pub host: String,
    pub addresses: Vec<SocketAddr>,
    pub expires_at_unix_ms: u64,
    pub cache_hit: bool,
}

#[derive(Debug)]
struct CachedDns {
    addresses: Vec<IpAddr>,
    expires_at: Instant,
    expires_at_unix_ms: u64,
}

#[derive(Debug, Default)]
struct DnsState {
    entries: HashMap<String, CachedDns>,
}

#[derive(Debug)]
pub struct NetworkInfrastructure {
    config: NetworkConfig,
    state: Mutex<DnsState>,
    tls_profile: TlsProfile,
}

impl NetworkInfrastructure {
    pub fn new(config: NetworkConfig) -> Result<Self, VrtCError> {
        if config.max_cache_entries == 0
            || config.max_cache_entries > 100_000
            || config.max_addresses_per_host == 0
            || config.max_addresses_per_host > 32
            || config.cache_ttl_ms == 0
            || config.cache_ttl_ms > 300_000
            || config.lookup_timeout_ms == 0
            || config.lookup_timeout_ms > 30_000
        {
            return Err(network_error(
                "INVALID_NETWORK_CONFIG",
                "DNS cache, address, TTL, and lookup timeout limits must be positive and bounded",
                ErrorCategory::Validation,
            ));
        }
        Ok(Self {
            config,
            state: Mutex::new(DnsState::default()),
            tls_profile: TlsProfile::default(),
        })
    }

    /// Resolve a host through the system resolver and reject non-public answers.
    /// Returned socket addresses are suitable for pinning a single HTTP hop.
    pub async fn resolve_public_addresses(
        &self,
        host: &str,
        port: u16,
    ) -> Result<DnsResolution, VrtCError> {
        let host = normalize_host(host);
        if host.is_empty()
            || host.len() > 253
            || host.chars().any(char::is_control)
            || host.chars().any(char::is_whitespace)
            || !matches!(port, 80 | 443)
        {
            return Err(network_error(
                "INVALID_DNS_REQUEST",
                "DNS host must be valid and the port must be HTTP or HTTPS",
                ErrorCategory::Validation,
            ));
        }

        if let Ok(address) = host.parse::<IpAddr>() {
            if !is_public_ip(address) {
                return Err(network_error(
                    "NON_PUBLIC_DNS_ADDRESS",
                    "The requested IP address is not permitted for public fetching",
                    ErrorCategory::PolicyDenied,
                ));
            }
            let now_ms = unix_time_ms();
            return Ok(DnsResolution {
                host,
                addresses: vec![SocketAddr::new(address, port)],
                expires_at_unix_ms: now_ms.saturating_add(self.config.cache_ttl_ms),
                cache_hit: false,
            });
        }

        if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
            return Err(network_error(
                "LOCAL_DNS_NAME_BLOCKED",
                "Local-only hostnames are not permitted for public fetching",
                ErrorCategory::PolicyDenied,
            ));
        }

        if let Some(cached) = self.cached_addresses(&host)? {
            return Ok(DnsResolution {
                host,
                addresses: cached
                    .addresses
                    .into_iter()
                    .map(|address| SocketAddr::new(address, port))
                    .collect(),
                expires_at_unix_ms: cached.expires_at_unix_ms,
                cache_hit: true,
            });
        }

        let lookup = timeout(
            Duration::from_millis(self.config.lookup_timeout_ms),
            tokio::net::lookup_host((host.as_str(), port)),
        )
        .await
        .map_err(|_| {
            network_error(
                "DNS_LOOKUP_TIMEOUT",
                "System DNS resolution exceeded the configured timeout",
                ErrorCategory::Timeout,
            )
        })?
        .map_err(|_| {
            network_error(
                "DNS_LOOKUP_FAILED",
                "The system resolver could not resolve the requested hostname",
                ErrorCategory::Unavailable,
            )
        })?;

        let addresses: BTreeSet<IpAddr> = lookup.map(|address| address.ip()).collect();
        if addresses.is_empty() || addresses.len() > self.config.max_addresses_per_host {
            return Err(network_error(
                "DNS_ADDRESS_LIMIT",
                "The DNS response was empty or exceeded the configured address limit",
                ErrorCategory::ResourceExhausted,
            ));
        }
        if addresses.iter().any(|address| !is_public_ip(*address))
        {
            return Err(network_error(
                "NON_PUBLIC_DNS_ADDRESS",
                "The hostname resolves to at least one address not permitted for public fetching",
                ErrorCategory::PolicyDenied,
            ));
        }

        let expires_at = Instant::now() + Duration::from_millis(self.config.cache_ttl_ms);
        let expires_at_unix_ms = unix_time_ms().saturating_add(self.config.cache_ttl_ms);
        let cached_addresses: Vec<IpAddr> = addresses.into_iter().collect();
        self.store_addresses(&host, cached_addresses.clone(), expires_at, expires_at_unix_ms)?;

        Ok(DnsResolution {
            host,
            addresses: cached_addresses
                .into_iter()
                .map(|address| SocketAddr::new(address, port))
                .collect(),
            expires_at_unix_ms,
            cache_hit: false,
        })
    }

    /// Apply VRTc's mandatory TLS floor and certificate-validation defaults.
    /// Hostname verification remains enabled by reqwest's rustls verifier.
    #[must_use]
    pub fn apply_tls_policy(&self, builder: ClientBuilder) -> ClientBuilder {
        builder
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
            .danger_accept_invalid_certs(false)
    }

    #[must_use]
    pub fn tls_profile(&self) -> TlsProfile {
        self.tls_profile.clone()
    }

    pub fn cache_entries(&self) -> Result<usize, VrtCError> {
        Ok(self.lock_state()?.entries.len())
    }

    fn cached_addresses(&self, host: &str) -> Result<Option<CachedDns>, VrtCError> {
        let mut state = self.lock_state()?;
        if let Some(entry) = state.entries.get(host) {
            if entry.expires_at > Instant::now() {
                return Ok(Some(CachedDns {
                    addresses: entry.addresses.clone(),
                    expires_at: entry.expires_at,
                    expires_at_unix_ms: entry.expires_at_unix_ms,
                }));
            }
        }
        state.entries.remove(host);
        Ok(None)
    }

    fn store_addresses(
        &self,
        host: &str,
        addresses: Vec<IpAddr>,
        expires_at: Instant,
        expires_at_unix_ms: u64,
    ) -> Result<(), VrtCError> {
        let mut state = self.lock_state()?;
        if !state.entries.contains_key(host)
            && state.entries.len() >= self.config.max_cache_entries
        {
            if let Some(oldest) = state
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.expires_at)
                .map(|(key, _)| key.clone())
            {
                state.entries.remove(&oldest);
            }
        }
        state.entries.insert(
            host.to_owned(),
            CachedDns {
                addresses,
                expires_at,
                expires_at_unix_ms,
            },
        );
        Ok(())
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, DnsState>, VrtCError> {
        self.state.lock().map_err(|_| {
            network_error(
                "DNS_CACHE_LOCK_POISONED",
                "The DNS cache lock is unavailable",
                ErrorCategory::Internal,
            )
        })
    }
}

fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_documentation()
                || octets[0] == 0
                || octets[0] >= 240
                || (octets[0] == 100 && (octets[1] & 0xc0) == 64)
                || (octets[0] == 192 && octets[1] == 0)
                || (octets[0] == 198 && matches!(octets[1], 18 | 19)))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            let global_unicast = segments[0] & 0xe000 == 0x2000;
            global_unicast
                && !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        }
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn network_error(code: &str, message: &str, category: ErrorCategory) -> VrtCError {
    VrtCError::new(code, message, category, false).for_component("engine.vrtc.network")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_network_limits_and_exposes_safe_tls_profile() {
        let engine = NetworkInfrastructure::new(NetworkConfig::default()).unwrap();
        let tls = engine.tls_profile();
        assert_eq!(tls.minimum_version, TlsVersion::Tls12);
        assert!(tls.certificate_validation);
        assert!(tls.hostname_validation);
        assert!(tls.https_downgrade_blocked);
        assert!(!tls.invalid_certificates_allowed);
        assert_eq!(engine.cache_entries().unwrap(), 0);
    }

    #[test]
    fn rejects_zero_cache_limits() {
        let config = NetworkConfig {
            max_cache_entries: 0,
            ..NetworkConfig::default()
        };
        assert_eq!(
            NetworkInfrastructure::new(config).unwrap_err().code,
            "INVALID_NETWORK_CONFIG"
        );
    }

    #[tokio::test]
    async fn blocks_local_and_non_public_dns_targets_before_lookup() {
        let engine = NetworkInfrastructure::new(NetworkConfig::default()).unwrap();
        assert_eq!(
            engine
                .resolve_public_addresses("127.0.0.1", 443)
                .await
                .unwrap_err()
                .code,
            "NON_PUBLIC_DNS_ADDRESS"
        );
        assert_eq!(
            engine
                .resolve_public_addresses("localhost", 443)
                .await
                .unwrap_err()
                .code,
            "LOCAL_DNS_NAME_BLOCKED"
        );
    }
}
