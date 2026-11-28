//! Native, policy-constrained HTTP fetcher for VRTc.
//!
//! Requests are limited to public HTTP(S) endpoints, DNS results are checked
//! and pinned for each hop, redirects are revalidated, and response bodies are
//! bounded. The fetcher does not crawl, render JavaScript, or bypass access
//! controls; those concerns belong to separate VRTc engines.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use reqwest::header::{ACCEPT, CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use reqwest::redirect::Policy;
use reqwest::{Client, StatusCode};
use url::Url;
use vrtc_core::{ErrorCategory, VrtCError};
use vrtc_network::{NetworkConfig, NetworkEngine, RouteMode, TlsMinimumVersion};

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
    network: Arc<NetworkEngine>,
}

impl FetchEngine {
    pub fn new(config: FetchConfig) -> Result<Self, VrtCError> {
        Self::with_network(
            config,
            Arc::new(NetworkEngine::new(NetworkConfig::default())?),
        )
    }

    /// Create a Fetcher that shares the runtime's configured network policy.
    pub fn with_network(
        config: FetchConfig,
        network: Arc<NetworkEngine>,
    ) -> Result<Self, VrtCError> {
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
        Ok(Self { config, network })
    }

    async fn resolve_public_endpoint(&self, url: &Url) -> Result<(String, SocketAddr), VrtCError> {
        let host = url
            .host_str()
            .ok_or_else(|| {
                fetch_error(
                    "FETCH_HOST_MISSING",
                    "Fetch URL does not contain a hostname",
                    ErrorCategory::Validation,
                    false,
                )
            })?
            .to_ascii_lowercase();
        let port = url.port_or_known_default().ok_or_else(|| {
            fetch_error(
                "FETCH_PORT_MISSING",
                "Fetch URL does not contain a valid port",
                ErrorCategory::Validation,
                false,
            )
        })?;

        let resolution = self.network.resolve_public(&host, port).await?;
        let address = resolution.addresses.into_iter().next().ok_or_else(|| {
            fetch_error(
                "FETCH_DNS_EMPTY",
                "The network engine returned no validated addresses",
                ErrorCategory::PolicyDenied,
                false,
            )
        })?;
        Ok((host, address))
    }

    /// Fetch one public HTTP(S) resource with bounded redirects and body size.
    pub async fn fetch(&self, raw_url: &str) -> Result<FetchResponse, VrtCError> {
        let requested = validate_url(raw_url)?;
        let requested_url = requested.to_string();
        let mut current = requested;
        let mut redirect_chain = Vec::new();

        for hop in 0..=self.config.max_redirects {
            let route = self.network.plan_route(current.as_str())?;
            if !matches!(route.mode, RouteMode::Direct) {
                return Err(fetch_error(
                    "PROXY_TRANSPORT_NOT_AVAILABLE",
                    "The selected proxy route cannot be applied safely by this Fetcher transport",
                    ErrorCategory::PolicyDenied,
                    false,
                ));
            }
            let (host, address) = self.resolve_public_endpoint(&current).await?;
            let client = http_client_builder(
                self.config.timeout,
                &self.config.user_agent,
                &host,
                address,
                route.tls,
            )
            .build()
            .map_err(|_| {
                fetch_error(
                    "HTTP_CLIENT_INIT_FAILED",
                    "The HTTP client could not be initialized",
                    ErrorCategory::Internal,
                    true,
                )
            })?;

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
                    .ok_or_else(|| {
                        fetch_error(
                            "INVALID_REDIRECT",
                            "A redirect response did not contain a valid Location header",
                            ErrorCategory::Validation,
                            false,
                        )
                    })?;
                let next = current.join(location).map_err(|_| {
                    fetch_error(
                        "INVALID_REDIRECT_URL",
                        "The redirect target is not a valid URL",
                        ErrorCategory::Validation,
                        false,
                    )
                })?;
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
            while let Some(chunk) = response.chunk().await.map_err(|_| {
                fetch_error(
                    "HTTP_BODY_READ_FAILED",
                    "The response body could not be read completely",
                    ErrorCategory::Internal,
                    true,
                )
            })? {
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

fn http_client_builder(
    timeout: Duration,
    user_agent: &str,
    host: &str,
    address: SocketAddr,
    tls_minimum: TlsMinimumVersion,
) -> reqwest::ClientBuilder {
    Client::builder()
        .timeout(timeout)
        .redirect(Policy::none())
        .no_proxy()
        .min_tls_version(reqwest_tls_version(tls_minimum))
        .user_agent(user_agent)
        .resolve(host, address)
}

fn reqwest_tls_version(minimum: TlsMinimumVersion) -> reqwest::tls::Version {
    match minimum {
        TlsMinimumVersion::Tls12 => reqwest::tls::Version::TLS_1_2,
        TlsMinimumVersion::Tls13 => reqwest::tls::Version::TLS_1_3,
    }
}

fn validate_url(raw_url: &str) -> Result<Url, VrtCError> {
    let url = Url::parse(raw_url).map_err(|_| {
        fetch_error(
            "INVALID_FETCH_URL",
            "Fetch URL must be an absolute HTTP or HTTPS URL",
            ErrorCategory::Validation,
            false,
        )
    })?;
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
    [
        "content-type",
        "content-language",
        "last-modified",
        "etag",
        "cache-control",
    ]
    .into_iter()
    .filter_map(|name| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(|value| (name.to_owned(), value.to_owned()))
    })
    .collect()
}

fn fetch_error(code: &str, message: &str, category: ErrorCategory, retryable: bool) -> VrtCError {
    VrtCError::new(code, message, category, retryable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::UNIX_EPOCH;

    struct TestTlsServer {
        child: Child,
        address: SocketAddr,
        root_certificate_path: PathBuf,
        directory: PathBuf,
    }

    impl Drop for TestTlsServer {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    fn start_tls_server(version_flag: &str) -> TestTlsServer {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("vrtc-fetcher-tls-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&directory).expect("test TLS directory should be created");

        let root_certificate_path = directory.join("test-ca.pem");
        let root_key_path = directory.join("test-ca-key.pem");
        let server_certificate_path = directory.join("localhost-cert.pem");
        let server_key_path = directory.join("localhost-key.pem");
        let server_request_path = directory.join("localhost.csr");
        let extensions_path = directory.join("localhost.ext");

        let root = Command::new("openssl")
            .args(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout"])
            .arg(&root_key_path)
            .arg("-out")
            .arg(&root_certificate_path)
            .args([
                "-days",
                "2",
                "-sha256",
                "-subj",
                "/CN=VRTc-Test-CA",
                "-addext",
                "basicConstraints=critical,CA:TRUE",
                "-addext",
                "keyUsage=critical,keyCertSign,cRLSign",
            ])
            .output()
            .expect("OpenSSL CLI is required for local TLS handshake tests");
        assert!(
            root.status.success(),
            "OpenSSL could not generate the test CA: {}",
            String::from_utf8_lossy(&root.stderr)
        );

        let request = Command::new("openssl")
            .args(["req", "-new", "-newkey", "rsa:2048", "-nodes", "-keyout"])
            .arg(&server_key_path)
            .arg("-out")
            .arg(&server_request_path)
            .args(["-subj", "/CN=localhost"])
            .output()
            .expect("OpenSSL should generate the test server key");
        assert!(
            request.status.success(),
            "OpenSSL could not generate the test server request: {}",
            String::from_utf8_lossy(&request.stderr)
        );

        fs::write(
            &extensions_path,
            "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost\n",
        )
        .expect("test certificate extensions should be written");
        let signed = Command::new("openssl")
            .arg("x509")
            .arg("-req")
            .arg("-in")
            .arg(&server_request_path)
            .arg("-CA")
            .arg(&root_certificate_path)
            .arg("-CAkey")
            .arg(&root_key_path)
            .arg("-CAcreateserial")
            .arg("-out")
            .arg(&server_certificate_path)
            .args(["-days", "2", "-sha256", "-extfile"])
            .arg(&extensions_path)
            .output()
            .expect("OpenSSL should sign the test server certificate");
        assert!(
            signed.status.success(),
            "OpenSSL could not sign the test server certificate: {}",
            String::from_utf8_lossy(&signed.stderr)
        );

        let listener =
            TcpListener::bind(("127.0.0.1", 0)).expect("test server port should be available");
        let address = listener
            .local_addr()
            .expect("test server address should be available");
        drop(listener);

        let mut child = Command::new("openssl")
            .arg("s_server")
            .arg("-accept")
            .arg(format!("127.0.0.1:{}", address.port()))
            .arg("-cert")
            .arg(&server_certificate_path)
            .arg("-key")
            .arg(&server_key_path)
            .args(["-www", "-quiet", version_flag, "-naccept", "2"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("OpenSSL TLS test server should start");

        let mut ready = false;
        for _ in 0..100 {
            if child
                .try_wait()
                .expect("test server status should be readable")
                .is_some()
            {
                panic!("OpenSSL TLS test server exited before accepting connections");
            }
            if TcpStream::connect(address).is_ok() {
                ready = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(ready, "OpenSSL TLS test server did not become ready");

        TestTlsServer {
            child,
            address,
            root_certificate_path,
            directory,
        }
    }

    async fn request_from_test_server_with_identity(
        server: &TestTlsServer,
        minimum: TlsMinimumVersion,
        hostname: &str,
        trust_test_ca: bool,
    ) -> Result<StatusCode, reqwest::Error> {
        let builder = http_client_builder(
            Duration::from_secs(5),
            "VRTc-Fetcher-TLS-Test/0.1",
            hostname,
            server.address,
            minimum,
        );

        let builder = if trust_test_ca {
            let certificate = reqwest::Certificate::from_pem(
                &fs::read(&server.root_certificate_path)
                    .expect("test root certificate should be readable"),
            )
            .expect("test certificate should parse");
            builder.add_root_certificate(certificate)
        } else {
            builder
        };

        let client = builder.build().expect("test client should build");
        client
            .get(format!("https://{hostname}:{}/", server.address.port()))
            .send()
            .await
            .map(|response| response.status())
    }

    async fn request_from_test_server(
        server: &TestTlsServer,
        minimum: TlsMinimumVersion,
    ) -> Result<StatusCode, reqwest::Error> {
        request_from_test_server_with_identity(server, minimum, "localhost", true).await
    }

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
    fn network_tls_minimum_maps_to_reqwest_tls_versions() {
        assert_eq!(
            reqwest_tls_version(TlsMinimumVersion::Tls12),
            reqwest::tls::Version::TLS_1_2
        );
        assert_eq!(
            reqwest_tls_version(TlsMinimumVersion::Tls13),
            reqwest::tls::Version::TLS_1_3
        );
    }

    #[test]
    fn reqwest_client_accepts_both_supported_tls_minimums() {
        for minimum in [TlsMinimumVersion::Tls12, TlsMinimumVersion::Tls13] {
            Client::builder()
                .min_tls_version(reqwest_tls_version(minimum))
                .build()
                .expect("configured rustls TLS minimum should build");
        }
    }

    #[tokio::test]
    async fn minimum_tls_is_enforced_during_real_handshakes() {
        let tls12_server = start_tls_server("-tls1_2");
        assert_eq!(
            request_from_test_server(&tls12_server, TlsMinimumVersion::Tls12)
                .await
                .expect("TLS 1.2 handshake should succeed"),
            StatusCode::OK,
            "a TLS 1.2 minimum should accept a TLS 1.2 server"
        );
        drop(tls12_server);

        let tls12_server = start_tls_server("-tls1_2");
        assert!(
            request_from_test_server(&tls12_server, TlsMinimumVersion::Tls13)
                .await
                .is_err(),
            "a TLS 1.3 minimum must reject a TLS 1.2-only server"
        );
        drop(tls12_server);

        let tls13_server = start_tls_server("-tls1_3");
        assert_eq!(
            request_from_test_server(&tls13_server, TlsMinimumVersion::Tls13)
                .await
                .expect("TLS 1.3 handshake should succeed"),
            StatusCode::OK,
            "a TLS 1.3 minimum should accept a TLS 1.3 server"
        );
    }

    #[tokio::test]
    async fn certificate_and_hostname_validation_remain_enforced() {
        let untrusted_server = start_tls_server("-tls1_3");
        assert!(
            request_from_test_server_with_identity(
                &untrusted_server,
                TlsMinimumVersion::Tls13,
                "localhost",
                false,
            )
            .await
            .is_err(),
            "a valid TLS certificate from an untrusted test CA must be rejected"
        );
        drop(untrusted_server);

        let mismatched_hostname_server = start_tls_server("-tls1_3");
        assert!(
            request_from_test_server_with_identity(
                &mismatched_hostname_server,
                TlsMinimumVersion::Tls13,
                "wrong.localhost",
                true,
            )
            .await
            .is_err(),
            "a trusted certificate for localhost must not authenticate another hostname"
        );
    }

    #[test]
    fn fetcher_accepts_the_runtime_network_engine() {
        let network = Arc::new(NetworkEngine::new(NetworkConfig::default()).unwrap());
        assert!(FetchEngine::with_network(FetchConfig::default(), network).is_ok());
    }
}
