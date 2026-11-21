//! Isolated browser rendering of already-fetched HTML snapshots.
//!
//! The renderer never navigates to the source URL. It loads the supplied HTML
//! into an about:blank page, disables JavaScript, and injects a restrictive CSP
//! so rendering cannot fetch remote subresources or execute page scripts.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chromiumoxide::browser::{Browser, BrowserConfig};
use futures::StreamExt;
use tokio::time::timeout;
use vrtc_core::{ErrorCategory, VrtCError};
use vrtc_policy::{PolicyEngine, PolicyOutcome, PolicyRequest};

#[derive(Debug, Clone)]
pub struct BrowserRenderConfig {
    pub chrome_executable: Option<PathBuf>,
    pub operation_timeout_ms: u64,
    pub max_html_bytes: usize,
}

impl Default for BrowserRenderConfig {
    fn default() -> Self {
        Self {
            chrome_executable: None,
            operation_timeout_ms: 20_000,
            max_html_bytes: 2 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserRenderRequest {
    pub source_url: String,
    pub origin_url: Option<String>,
    pub html: String,
    pub is_crawl: bool,
    pub crawl_depth: u8,
    pub robots_allowed: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPage {
    pub source_url: String,
    pub title: Option<String>,
    pub html: String,
    pub html_bytes: usize,
    pub elapsed_ms: u128,
}

pub struct BrowserRenderer {
    config: BrowserRenderConfig,
    policy: Arc<PolicyEngine>,
}

impl BrowserRenderer {
    pub fn new(config: BrowserRenderConfig, policy: Arc<PolicyEngine>) -> Result<Self, VrtCError> {
        if config.operation_timeout_ms == 0 || config.max_html_bytes == 0 {
            return Err(browser_error(
                "INVALID_BROWSER_CONFIG",
                "Browser timeout and HTML size limits must be positive",
                ErrorCategory::Validation,
            ));
        }
        Ok(Self { config, policy })
    }

    pub async fn render(&self, request: BrowserRenderRequest) -> Result<RenderedPage, VrtCError> {
        if request.html.len() > self.config.max_html_bytes {
            return Err(browser_error(
                "BROWSER_INPUT_TOO_LARGE",
                "Fetched HTML exceeds the configured input limit",
                ErrorCategory::ResourceExhausted,
            ));
        }

        let policy = self.policy.evaluate(&PolicyRequest {
            url: request.source_url.clone(),
            origin_url: request.origin_url.clone(),
            is_crawl: request.is_crawl,
            crawl_depth: request.crawl_depth,
            robots_allowed: request.robots_allowed,
        })?;
        if policy.outcome != PolicyOutcome::Allow {
            return Err(browser_error(
                "BROWSER_POLICY_DENIED",
                &format!("Browser rendering rejected by policy: {:?}", policy.reason),
                ErrorCategory::Validation,
            ));
        }
        let source_url = policy.normalized_url.ok_or_else(|| {
            browser_error(
                "BROWSER_POLICY_RESULT_INVALID",
                "Allowed policy result did not include a normalized source URL",
                ErrorCategory::Internal,
            )
        })?;

        let started = Instant::now();
        let browser_config = match self.config.chrome_executable.as_ref() {
            Some(executable) => BrowserConfig::builder()
                .chrome_executable(executable.clone())
                .build(),
            None => BrowserConfig::builder().build(),
        }
        .map_err(|error| {
            browser_error(
                "BROWSER_CONFIG_BUILD_FAILED",
                &error.to_string(),
                ErrorCategory::Validation,
            )
        })?;

        let operation_timeout = Duration::from_millis(self.config.operation_timeout_ms);
        let (mut browser, mut handler) =
            timeout(operation_timeout, Browser::launch(browser_config))
                .await
                .map_err(|_| {
                    browser_error(
                        "BROWSER_LAUNCH_TIMEOUT",
                        "Browser launch exceeded the configured timeout",
                        ErrorCategory::ResourceExhausted,
                    )
                })?
                .map_err(|error| {
                    browser_error(
                        "BROWSER_LAUNCH_FAILED",
                        &error.to_string(),
                        ErrorCategory::Internal,
                    )
                })?;

        let handler_task = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    break;
                }
            }
        });

        let page_result = timeout(operation_timeout, browser.new_page("about:blank")).await;
        let page = match page_result {
            Ok(Ok(page)) => page,
            Ok(Err(error)) => {
                let _ = browser.close().await;
                handler_task.abort();
                return Err(browser_error(
                    "BROWSER_PAGE_CREATE_FAILED",
                    &error.to_string(),
                    ErrorCategory::Internal,
                ));
            }
            Err(_) => {
                let _ = browser.close().await;
                handler_task.abort();
                return Err(browser_error(
                    "BROWSER_PAGE_TIMEOUT",
                    "Creating the isolated browser page exceeded the configured timeout",
                    ErrorCategory::ResourceExhausted,
                ));
            }
        };

        let offline_html = apply_offline_csp(&request.html);
        let render_result = async {
            timeout(operation_timeout, page.disable_javascript())
                .await
                .map_err(|_| {
                    browser_error(
                        "BROWSER_SCRIPT_POLICY_TIMEOUT",
                        "Disabling JavaScript exceeded the configured timeout",
                        ErrorCategory::ResourceExhausted,
                    )
                })?
                .map_err(|error| {
                    browser_error(
                        "BROWSER_SCRIPT_POLICY_FAILED",
                        &error.to_string(),
                        ErrorCategory::Internal,
                    )
                })?;

            timeout(operation_timeout, page.set_content(offline_html))
                .await
                .map_err(|_| {
                    browser_error(
                        "BROWSER_RENDER_TIMEOUT",
                        "Rendering the HTML snapshot exceeded the configured timeout",
                        ErrorCategory::ResourceExhausted,
                    )
                })?
                .map_err(|error| {
                    browser_error(
                        "BROWSER_RENDER_FAILED",
                        &error.to_string(),
                        ErrorCategory::Internal,
                    )
                })?;

            let html = timeout(operation_timeout, page.content())
                .await
                .map_err(|_| {
                    browser_error(
                        "BROWSER_CONTENT_TIMEOUT",
                        "Reading rendered HTML exceeded the configured timeout",
                        ErrorCategory::ResourceExhausted,
                    )
                })?
                .map_err(|error| {
                    browser_error(
                        "BROWSER_CONTENT_FAILED",
                        &error.to_string(),
                        ErrorCategory::Internal,
                    )
                })?;

            if html.len() > self.config.max_html_bytes {
                return Err(browser_error(
                    "BROWSER_OUTPUT_TOO_LARGE",
                    "Rendered HTML exceeds the configured output limit",
                    ErrorCategory::ResourceExhausted,
                ));
            }

            let title = timeout(operation_timeout, page.get_title())
                .await
                .ok()
                .and_then(Result::ok)
                .flatten();

            Ok(RenderedPage {
                source_url,
                title,
                html_bytes: html.len(),
                html,
                elapsed_ms: started.elapsed().as_millis(),
            })
        }
        .await;

        let _ = browser.close().await;
        let _ = handler_task.await;
        render_result
    }
}

fn apply_offline_csp(html: &str) -> String {
    const CSP: &str = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:; script-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; media-src 'none'; form-action 'none'; base-uri 'none\">";
    let lower = html.to_ascii_lowercase();

    if let Some(head_start) = lower.find("<head") {
        if let Some(head_end) = lower[head_start..].find('>') {
            let insert_at = head_start + head_end + 1;
            return format!("{}{}{}", &html[..insert_at], CSP, &html[insert_at..]);
        }
    }
    if let Some(html_start) = lower.find("<html") {
        if let Some(html_end) = lower[html_start..].find('>') {
            let insert_at = html_start + html_end + 1;
            return format!(
                "{}<head>{CSP}</head>{}",
                &html[..insert_at],
                &html[insert_at..]
            );
        }
    }
    format!("<html><head>{CSP}</head><body>{html}</body></html>")
}

fn browser_error(code: &str, message: &str, category: ErrorCategory) -> VrtCError {
    VrtCError::new(code, message, category, false).for_component("engine.vrtc.browser")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer() -> BrowserRenderer {
        BrowserRenderer::new(
            BrowserRenderConfig::default(),
            Arc::new(PolicyEngine::new(Default::default()).unwrap()),
        )
        .unwrap()
    }

    #[test]
    fn injects_offline_policy_before_page_content() {
        let html = "<html><head><title>Offline</title></head><body>snapshot</body></html>";
        let safe = apply_offline_csp(html);
        assert!(safe.contains("default-src 'none'"));
        assert!(safe.find("Content-Security-Policy").unwrap() < safe.find("<title>").unwrap());
    }

    #[test]
    fn wraps_fragment_documents_with_offline_policy() {
        let safe = apply_offline_csp("<h1>Snapshot</h1>");
        assert!(safe.starts_with("<html><head>"));
        assert!(safe.contains("<h1>Snapshot</h1>"));
    }

    #[tokio::test]
    async fn policy_denial_happens_before_browser_launch() {
        let error = renderer()
            .render(BrowserRenderRequest {
                source_url: "file:///etc/passwd".to_owned(),
                origin_url: None,
                html: "<h1>nope</h1>".to_owned(),
                is_crawl: false,
                crawl_depth: 0,
                robots_allowed: None,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, "BROWSER_POLICY_DENIED");
    }

    #[tokio::test]
    async fn crawl_requires_robots_approval_before_browser_launch() {
        let error = renderer()
            .render(BrowserRenderRequest {
                source_url: "https://example.org".to_owned(),
                origin_url: None,
                html: "<h1>snapshot</h1>".to_owned(),
                is_crawl: true,
                crawl_depth: 1,
                robots_allowed: None,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, "BROWSER_POLICY_DENIED");
    }
}
