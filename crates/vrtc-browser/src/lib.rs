//! Isolated browser rendering of already-fetched HTML snapshots.
//!
//! The renderer never navigates to the source URL. It loads the supplied HTML
//! into an about:blank page, disables JavaScript, and injects a restrictive CSP
//! so rendering cannot fetch remote subresources or execute page scripts.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::emulation::SetScriptExecutionDisabledParams;
use futures::StreamExt;
use tokio::sync::{oneshot, watch, Semaphore};
use tokio::time::{timeout, timeout_at};
use vrtc_core::{ErrorCategory, VrtCError};
use vrtc_policy::{PolicyEngine, PolicyOutcome, PolicyRequest};

#[derive(Debug, Clone)]
pub struct BrowserRenderConfig {
    pub chrome_executable: Option<PathBuf>,
    pub operation_timeout_ms: u64,
    pub max_html_bytes: usize,
    pub max_concurrent_renders: usize,
    pub max_queue_wait_ms: u64,
}

impl Default for BrowserRenderConfig {
    fn default() -> Self {
        Self {
            chrome_executable: None,
            operation_timeout_ms: 20_000,
            max_html_bytes: 2 * 1024 * 1024,
            max_concurrent_renders: 2,
            max_queue_wait_ms: 5_000,
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
    render_slots: Arc<Semaphore>,
    supervisor: Arc<BrowserSupervisor>,
}

impl BrowserRenderer {
    pub fn new(config: BrowserRenderConfig, policy: Arc<PolicyEngine>) -> Result<Self, VrtCError> {
        if config.operation_timeout_ms == 0
            || config.max_html_bytes == 0
            || config.max_concurrent_renders == 0
            || config.max_concurrent_renders > Semaphore::MAX_PERMITS
            || config.max_queue_wait_ms == 0
        {
            return Err(browser_error(
                "INVALID_BROWSER_CONFIG",
                "Browser timeout, HTML size, concurrency, and queue wait limits must be positive",
                ErrorCategory::Validation,
            ));
        }
        let render_slots = Arc::new(Semaphore::new(config.max_concurrent_renders));
        Ok(Self {
            config,
            policy,
            render_slots,
            supervisor: Arc::new(BrowserSupervisor::new()),
        })
    }

    /// Stop new browser work and wait for all admitted render supervisors.
    ///
    /// Hosts must await this method before shutting down their Tokio runtime.
    /// It is idempotent; after it begins, this renderer permanently rejects new work.
    pub async fn shutdown(&self) -> Result<(), VrtCError> {
        self.supervisor.shutdown().await
    }

    /// Admit work before spawning so queued requests do not create an
    /// unbounded number of detached Tokio tasks.
    pub async fn render(&self, request: BrowserRenderRequest) -> Result<RenderedPage, VrtCError> {
        self.supervisor.ensure_open()?;
        let source_url = self.validate_request(&request)?;

        // One deadline covers queue admission, launch, page creation, and rendering.
        // The permit is acquired by the caller before spawning, so the number of
        // supervised workers can never exceed configured render concurrency.
        let operation_timeout = Duration::from_millis(self.config.operation_timeout_ms);
        let deadline = tokio::time::Instant::now() + operation_timeout;
        let queue_timeout = Duration::from_millis(self.config.max_queue_wait_ms)
            .min(deadline.saturating_duration_since(tokio::time::Instant::now()));
        let render_permit = timeout(
            queue_timeout,
            Arc::clone(&self.render_slots).acquire_owned(),
        )
        .await
        .map_err(|_| {
            browser_error(
                "BROWSER_RENDER_CAPACITY_TIMEOUT",
                "Browser render capacity was unavailable before the queue wait limit",
                ErrorCategory::ResourceExhausted,
            )
        })?
        .map_err(|_| {
            browser_error(
                "BROWSER_RENDER_CAPACITY_CLOSED",
                "Browser render capacity is unavailable",
                ErrorCategory::Internal,
            )
        })?;

        let worker = Self {
            config: self.config.clone(),
            policy: Arc::clone(&self.policy),
            render_slots: Arc::clone(&self.render_slots),
            supervisor: Arc::clone(&self.supervisor),
        };
        Arc::clone(&self.supervisor)
            .spawn(async move {
                worker
                    .render_inner(request, source_url, deadline, render_permit)
                    .await
            })?
            .await
            .map_err(|_| {
                browser_error(
                    "BROWSER_RENDER_SUPERVISOR_FAILED",
                    "The supervised browser render task ended without returning a result",
                    ErrorCategory::Internal,
                )
            })?
    }

    fn validate_request(&self, request: &BrowserRenderRequest) -> Result<String, VrtCError> {
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
        policy.normalized_url.ok_or_else(|| {
            browser_error(
                "BROWSER_POLICY_RESULT_INVALID",
                "Allowed policy result did not include a normalized source URL",
                ErrorCategory::Internal,
            )
        })
    }

    async fn render_inner(
        &self,
        request: BrowserRenderRequest,
        source_url: String,
        deadline: tokio::time::Instant,
        _render_permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<RenderedPage, VrtCError> {
        // The caller admitted this request before spawning. Holding the owned
        // permit here bounds both active browser work and detached supervisors.
        let operation_timeout = Duration::from_millis(self.config.operation_timeout_ms);

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

        let (mut browser, mut handler) = timeout_at(deadline, Browser::launch(browser_config))
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

        let mut handler_task = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    break;
                }
            }
        });

        let page_result = timeout_at(deadline, browser.new_page("about:blank")).await;
        let page = match page_result {
            Ok(Ok(page)) => page,
            Ok(Err(error)) => {
                let _ = shutdown_browser(&mut browser, &mut handler_task, operation_timeout).await;
                return Err(browser_error(
                    "BROWSER_PAGE_CREATE_FAILED",
                    &error.to_string(),
                    ErrorCategory::Internal,
                ));
            }
            Err(_) => {
                let _ = shutdown_browser(&mut browser, &mut handler_task, operation_timeout).await;
                return Err(browser_error(
                    "BROWSER_PAGE_TIMEOUT",
                    "Creating the isolated browser page exceeded the configured timeout",
                    ErrorCategory::ResourceExhausted,
                ));
            }
        };

        let offline_html = apply_offline_csp(&request.html);
        let render_result = async {
            timeout_at(
                deadline,
                page.execute(SetScriptExecutionDisabledParams::new(true)),
            )
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

            timeout_at(deadline, page.set_content(offline_html))
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

            let html = timeout_at(deadline, page.content())
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

            let title = timeout_at(deadline, page.get_title())
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

        let cleanup_result =
            shutdown_browser(&mut browser, &mut handler_task, operation_timeout).await;
        match (render_result, cleanup_result) {
            (Err(render_error), _) => Err(render_error),
            (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
            (Ok(page), Ok(())) => Ok(page),
        }
    }
}

/// Tracks admitted browser workers so explicit shutdown can close admission
/// and wait for every worker, including workers whose caller was cancelled.
struct BrowserSupervisor {
    state: Mutex<SupervisorState>,
    active_tx: watch::Sender<usize>,
}

struct SupervisorState {
    closed: bool,
    active: usize,
}

struct ActiveWorkerGuard {
    supervisor: Arc<BrowserSupervisor>,
}

impl BrowserSupervisor {
    fn new() -> Self {
        let (active_tx, _) = watch::channel(0);
        Self {
            state: Mutex::new(SupervisorState {
                closed: false,
                active: 0,
            }),
            active_tx,
        }
    }

    fn ensure_open(&self) -> Result<(), VrtCError> {
        let state = self.state.lock().map_err(|_| {
            browser_error(
                "BROWSER_SUPERVISOR_LOCK_POISONED",
                "Browser supervisor state is unavailable",
                ErrorCategory::Internal,
            )
        })?;
        if state.closed {
            return Err(browser_error(
                "BROWSER_RENDERER_SHUTTING_DOWN",
                "Browser renderer is shutting down and no longer accepts work",
                ErrorCategory::ResourceExhausted,
            ));
        }
        Ok(())
    }

    fn spawn<T, F>(self: Arc<Self>, operation: F) -> Result<oneshot::Receiver<T>, VrtCError>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        {
            let mut state = self.state.lock().map_err(|_| {
                browser_error(
                    "BROWSER_SUPERVISOR_LOCK_POISONED",
                    "Browser supervisor state is unavailable",
                    ErrorCategory::Internal,
                )
            })?;
            if state.closed {
                return Err(browser_error(
                    "BROWSER_RENDERER_SHUTTING_DOWN",
                    "Browser renderer is shutting down and no longer accepts work",
                    ErrorCategory::ResourceExhausted,
                ));
            }
            state.active += 1;
            self.active_tx.send_replace(state.active);
        }

        let (result_tx, result_rx) = oneshot::channel();
        tokio::spawn(async move {
            let _active_guard = ActiveWorkerGuard {
                supervisor: Arc::clone(&self),
            };
            let result = operation.await;
            let _ = result_tx.send(result);
        });
        Ok(result_rx)
    }

    async fn shutdown(&self) -> Result<(), VrtCError> {
        // Subscribe before closing admission so no completion notification can
        // be lost between observing the active count and awaiting a change.
        let mut active_rx = self.active_tx.subscribe();
        {
            let mut state = self.state.lock().map_err(|_| {
                browser_error(
                    "BROWSER_SUPERVISOR_LOCK_POISONED",
                    "Browser supervisor state is unavailable",
                    ErrorCategory::Internal,
                )
            })?;
            state.closed = true;
            self.active_tx.send_replace(state.active);
        }

        loop {
            if *active_rx.borrow_and_update() == 0 {
                return Ok(());
            }
            active_rx.changed().await.map_err(|_| {
                browser_error(
                    "BROWSER_SUPERVISOR_SIGNAL_CLOSED",
                    "Browser supervisor completion signal closed unexpectedly",
                    ErrorCategory::Internal,
                )
            })?;
        }
    }

    fn worker_finished(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active = state.active.saturating_sub(1);
        self.active_tx.send_replace(state.active);
    }
}

impl Drop for ActiveWorkerGuard {
    fn drop(&mut self) {
        self.supervisor.worker_finished();
    }
}

/// Close the browser and stop its event handler without allowing cleanup to
/// wait forever. The handler is aborted even when the browser close operation
/// reaches its deadline.
async fn stop_browser_handler(
    handler_task: &mut tokio::task::JoinHandle<()>,
    cleanup_timeout: Duration,
) -> bool {
    handler_task.abort();
    timeout(cleanup_timeout, &mut *handler_task).await.is_ok()
}

async fn reap_browser_process(
    browser: &mut Browser,
    cleanup_timeout: Duration,
) -> Result<bool, VrtCError> {
    match timeout(cleanup_timeout, browser.wait()).await {
        Ok(Ok(Some(_))) => Ok(false),
        Ok(Ok(None)) => Ok(false),
        Ok(Err(error)) => {
            match timeout(cleanup_timeout, browser.kill()).await {
                Ok(Some(Ok(()))) => Ok(true),
                Ok(Some(Err(kill_error))) => Err(browser_error(
                    "BROWSER_PROCESS_KILL_FAILED",
                    &format!("Waiting for Chromium failed ({error}); forced termination also failed ({kill_error})"),
                    ErrorCategory::Internal,
                )),
                Ok(None) => Err(browser_error(
                    "BROWSER_PROCESS_WAIT_FAILED",
                    &error.to_string(),
                    ErrorCategory::Internal,
                )),
                Err(_) => Err(browser_error(
                    "BROWSER_PROCESS_KILL_TIMEOUT",
                    "Forced Chromium termination exceeded the configured cleanup timeout",
                    ErrorCategory::ResourceExhausted,
                )),
            }
        }
        Err(_) => match timeout(cleanup_timeout, browser.kill()).await {
            Ok(Some(Ok(()))) => Ok(true),
            Ok(Some(Err(error))) => Err(browser_error(
                "BROWSER_PROCESS_KILL_FAILED",
                &error.to_string(),
                ErrorCategory::Internal,
            )),
            Ok(None) => Ok(false),
            Err(_) => Err(browser_error(
                "BROWSER_PROCESS_KILL_TIMEOUT",
                "Chromium did not exit before the cleanup deadline and forced termination did not finish in time",
                ErrorCategory::ResourceExhausted,
            )),
        },
    }
}

async fn shutdown_browser(
    browser: &mut Browser,
    handler_task: &mut tokio::task::JoinHandle<()>,
    cleanup_timeout: Duration,
) -> Result<(), VrtCError> {
    let close_result = timeout(cleanup_timeout, browser.close()).await;
    let process_result = reap_browser_process(browser, cleanup_timeout).await;
    let handler_stopped = stop_browser_handler(handler_task, cleanup_timeout).await;

    match process_result {
        Err(error) => Err(error),
        Ok(forcibly_terminated) => match close_result {
            Err(_) => Err(browser_error(
                "BROWSER_CLEANUP_TIMEOUT",
                "Closing the browser exceeded the configured cleanup timeout; process reaping was still attempted",
                ErrorCategory::ResourceExhausted,
            )),
            Ok(Err(error)) => Err(browser_error(
                "BROWSER_CLEANUP_FAILED",
                &format!("Closing Chromium failed; process reaping was still attempted: {error}"),
                ErrorCategory::Internal,
            )),
            Ok(Ok(_)) if forcibly_terminated => Err(browser_error(
                "BROWSER_PROCESS_FORCIBLY_TERMINATED",
                "Chromium did not exit after graceful close and required forced termination",
                ErrorCategory::ResourceExhausted,
            )),
            Ok(Ok(_)) if !handler_stopped => Err(browser_error(
                "BROWSER_HANDLER_STOP_TIMEOUT",
                "Browser event handler did not stop within the configured cleanup timeout",
                ErrorCategory::ResourceExhausted,
            )),
            Ok(Ok(_)) => Ok(()),
        },
    }
}

/// Browser implementation family. Declaring a family does not imply that its
/// backend is installed or available at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserEngineFamily {
    Chromium,
    WebKit,
    Gecko,
    Custom,
}

/// Capabilities are explicit so callers do not assume feature parity between
/// browser engines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserEngineCapabilities {
    pub renders_fetched_snapshots: bool,
    pub javascript_control: bool,
    pub blocks_remote_subresources: bool,
    pub supports_live_navigation: bool,
}

/// Stable, non-secret metadata exposed by a browser backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserEngineInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub family: BrowserEngineFamily,
    pub capabilities: BrowserEngineCapabilities,
}

/// Backend-neutral browser rendering contract.
///
/// Implementations receive already-fetched HTML. The current contract does not
/// grant permission to navigate to arbitrary source URLs.
pub trait BrowserEngine: Send + Sync {
    fn info(&self) -> BrowserEngineInfo;

    fn render<'a>(
        &'a self,
        request: BrowserRenderRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RenderedPage, VrtCError>> + Send + 'a>>;
}

/// In-process engine catalog. Registration stores trusted Rust implementations;
/// it never loads plugins or executes code from descriptors.
#[derive(Default)]
pub struct BrowserEngineRegistry {
    engines: RwLock<HashMap<String, Arc<dyn BrowserEngine>>>,
}

impl BrowserEngineRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, engine: Arc<dyn BrowserEngine>) -> Result<(), VrtCError> {
        let info = engine.info();
        if info.id.trim().is_empty()
            || info.name.trim().is_empty()
            || info.version.trim().is_empty()
        {
            return Err(browser_error(
                "INVALID_BROWSER_ENGINE_IDENTITY",
                "Browser engine id, name, and version must not be empty",
                ErrorCategory::Validation,
            ));
        }
        let mut engines = self.engines.write().map_err(|_| {
            browser_error(
                "BROWSER_ENGINE_REGISTRY_LOCK_POISONED",
                "Browser engine registry lock is unavailable",
                ErrorCategory::Internal,
            )
        })?;
        if engines.contains_key(&info.id) {
            return Err(browser_error(
                "DUPLICATE_BROWSER_ENGINE",
                "A browser engine with this id is already registered",
                ErrorCategory::DuplicateId,
            ));
        }
        engines.insert(info.id, engine);
        Ok(())
    }

    pub fn engine_info(&self, id: &str) -> Result<BrowserEngineInfo, VrtCError> {
        let engines = self.engines.read().map_err(|_| {
            browser_error(
                "BROWSER_ENGINE_REGISTRY_LOCK_POISONED",
                "Browser engine registry lock is unavailable",
                ErrorCategory::Internal,
            )
        })?;
        engines.get(id).map(|engine| engine.info()).ok_or_else(|| {
            browser_error(
                "BROWSER_ENGINE_NOT_FOUND",
                "Requested browser engine is not registered",
                ErrorCategory::NotFound,
            )
        })
    }

    pub async fn render(
        &self,
        id: &str,
        request: BrowserRenderRequest,
    ) -> Result<RenderedPage, VrtCError> {
        let engine = {
            let engines = self.engines.read().map_err(|_| {
                browser_error(
                    "BROWSER_ENGINE_REGISTRY_LOCK_POISONED",
                    "Browser engine registry lock is unavailable",
                    ErrorCategory::Internal,
                )
            })?;
            engines.get(id).cloned().ok_or_else(|| {
                browser_error(
                    "BROWSER_ENGINE_NOT_FOUND",
                    "Requested browser engine is not registered",
                    ErrorCategory::NotFound,
                )
            })?
        };
        engine.render(request).await
    }
}

impl BrowserEngine for BrowserRenderer {
    fn info(&self) -> BrowserEngineInfo {
        BrowserEngineInfo {
            id: "browser.chromiumoxide".to_owned(),
            name: "Chromium snapshot renderer".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            family: BrowserEngineFamily::Chromium,
            capabilities: BrowserEngineCapabilities {
                renders_fetched_snapshots: true,
                javascript_control: true,
                blocks_remote_subresources: true,
                supports_live_navigation: false,
            },
        }
    }

    fn render<'a>(
        &'a self,
        request: BrowserRenderRequest,
    ) -> Pin<Box<dyn Future<Output = Result<RenderedPage, VrtCError>> + Send + 'a>> {
        Box::pin(BrowserRenderer::render(self, request))
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

    #[tokio::test]
    async fn supervisor_retains_capacity_and_waits_after_caller_drops_receiver() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let supervisor = Arc::new(BrowserSupervisor::new());
        let slots = Arc::new(Semaphore::new(1));
        let permit = Arc::clone(&slots).acquire_owned().await.unwrap();
        let worker_slots = Arc::clone(&slots);
        let completed = Arc::new(AtomicBool::new(false));
        let worker_completed = Arc::clone(&completed);
        let receiver = Arc::clone(&supervisor)
            .spawn(async move {
                let _permit = permit;
                tokio::time::sleep(Duration::from_millis(10)).await;
                worker_completed.store(true, Ordering::SeqCst);
            })
            .unwrap();

        drop(receiver);
        assert_eq!(slots.available_permits(), 0);
        supervisor.shutdown().await.unwrap();
        assert!(completed.load(Ordering::SeqCst));
        assert_eq!(worker_slots.available_permits(), 1);
    }

    #[tokio::test]
    async fn renderer_shutdown_is_idempotent_and_rejects_new_renders() {
        let renderer = renderer();
        renderer.shutdown().await.unwrap();
        renderer.shutdown().await.unwrap();

        let error = renderer
            .render(BrowserRenderRequest {
                source_url: "https://example.org".to_owned(),
                origin_url: None,
                html: "<h1>closed</h1>".to_owned(),
                is_crawl: false,
                crawl_depth: 0,
                robots_allowed: None,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, "BROWSER_RENDERER_SHUTTING_DOWN");
    }

    #[tokio::test]
    async fn shutdown_aborts_a_stalled_browser_handler() {
        let mut handler = tokio::spawn(async {
            std::future::pending::<()>().await;
        });
        assert!(stop_browser_handler(&mut handler, Duration::from_millis(100)).await);
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

#[cfg(test)]
mod engine_abstraction_tests {
    use super::*;

    fn renderer() -> BrowserRenderer {
        BrowserRenderer::new(
            BrowserRenderConfig::default(),
            Arc::new(PolicyEngine::new(Default::default()).unwrap()),
        )
        .unwrap()
    }

    #[test]
    fn rejects_zero_browser_concurrency_or_queue_wait() {
        let config = BrowserRenderConfig {
            max_concurrent_renders: 0,
            ..BrowserRenderConfig::default()
        };
        assert!(matches!(
            BrowserRenderer::new(
                config,
                Arc::new(PolicyEngine::new(Default::default()).unwrap()),
            ),
            Err(error) if error.code == "INVALID_BROWSER_CONFIG"
        ));

        let config = BrowserRenderConfig {
            max_queue_wait_ms: 0,
            ..BrowserRenderConfig::default()
        };
        assert!(matches!(
            BrowserRenderer::new(
                config,
                Arc::new(PolicyEngine::new(Default::default()).unwrap()),
            ),
            Err(error) if error.code == "INVALID_BROWSER_CONFIG"
        ));

        let config = BrowserRenderConfig {
            max_concurrent_renders: Semaphore::MAX_PERMITS + 1,
            ..BrowserRenderConfig::default()
        };
        assert!(matches!(
            BrowserRenderer::new(
                config,
                Arc::new(PolicyEngine::new(Default::default()).unwrap()),
            ),
            Err(error) if error.code == "INVALID_BROWSER_CONFIG"
        ));
    }

    #[tokio::test]
    async fn policy_denial_happens_before_render_capacity_admission() {
        let config = BrowserRenderConfig {
            max_concurrent_renders: 1,
            max_queue_wait_ms: 10,
            ..BrowserRenderConfig::default()
        };
        let renderer = BrowserRenderer::new(
            config,
            Arc::new(PolicyEngine::new(Default::default()).unwrap()),
        )
        .unwrap();
        let permit = Arc::clone(&renderer.render_slots)
            .acquire_owned()
            .await
            .unwrap();
        let error = renderer
            .render(BrowserRenderRequest {
                source_url: "file:///etc/passwd".to_owned(),
                origin_url: None,
                html: "<h1>blocked</h1>".to_owned(),
                is_crawl: false,
                crawl_depth: 0,
                robots_allowed: None,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, "BROWSER_POLICY_DENIED");
        drop(permit);
    }

    #[tokio::test]
    async fn render_queue_is_bounded_by_configured_wait_limit() {
        let config = BrowserRenderConfig {
            max_concurrent_renders: 1,
            max_queue_wait_ms: 10,
            ..BrowserRenderConfig::default()
        };
        let renderer = BrowserRenderer::new(
            config,
            Arc::new(PolicyEngine::new(Default::default()).unwrap()),
        )
        .unwrap();
        let permit = Arc::clone(&renderer.render_slots)
            .acquire_owned()
            .await
            .unwrap();
        let error = renderer
            .render(BrowserRenderRequest {
                source_url: "https://example.org".to_owned(),
                origin_url: None,
                html: "<h1>queued</h1>".to_owned(),
                is_crawl: false,
                crawl_depth: 0,
                robots_allowed: None,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, "BROWSER_RENDER_CAPACITY_TIMEOUT");
        drop(permit);
    }

    #[test]
    fn chromium_backend_declares_snapshot_only_capabilities() {
        let info = renderer().info();
        assert_eq!(info.id, "browser.chromiumoxide");
        assert_eq!(info.family, BrowserEngineFamily::Chromium);
        assert!(info.capabilities.renders_fetched_snapshots);
        assert!(info.capabilities.javascript_control);
        assert!(info.capabilities.blocks_remote_subresources);
        assert!(!info.capabilities.supports_live_navigation);
    }

    #[test]
    fn registry_rejects_duplicate_engine_ids() {
        let registry = BrowserEngineRegistry::new();
        let engine = Arc::new(renderer());
        registry.register(engine.clone()).unwrap();
        let error = registry.register(engine).unwrap_err();
        assert_eq!(error.code, "DUPLICATE_BROWSER_ENGINE");
    }

    #[test]
    fn registry_reports_unknown_engine_ids() {
        let registry = BrowserEngineRegistry::new();
        let error = registry.engine_info("browser.missing").unwrap_err();
        assert_eq!(error.code, "BROWSER_ENGINE_NOT_FOUND");
    }

    #[tokio::test]
    async fn registry_dispatch_preserves_browser_policy_gate() {
        let registry = BrowserEngineRegistry::new();
        registry.register(Arc::new(renderer())).unwrap();
        let error = registry
            .render(
                "browser.chromiumoxide",
                BrowserRenderRequest {
                    source_url: "file:///etc/passwd".to_owned(),
                    origin_url: None,
                    html: "<h1>blocked</h1>".to_owned(),
                    is_crawl: false,
                    crawl_depth: 0,
                    robots_allowed: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "BROWSER_POLICY_DENIED");
    }
}
