//! VRTc runtime façade joining component registration to executable engines.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use vrtc_archive::{ArchiveConfig, ArchiveEngine};
use vrtc_browser::{BrowserRenderConfig, BrowserRenderer};
use vrtc_core::{
    Capability, CapabilityAvailability, EngineDescriptor, ErrorCategory, LifecycleState,
    ProviderDescriptor, RegistrationSource, SearchRequest, SearchResponse, VrtCError,
};
use vrtc_crawler::{CrawlerConfig, CrawlerEngine};
use vrtc_delta::{DeltaConfig, DeltaEngine};
use vrtc_evidence::{EvidenceConfig, EvidenceEngine};
use vrtc_extract::{ExtractionConfig, ExtractionEngine};
use vrtc_feedback::{FeedbackConfig, FeedbackEngine};
use vrtc_fetcher::{FetchConfig, FetchEngine};
use vrtc_frontier::{FrontierConfig, FrontierEngine};
use vrtc_index::{IndexConfig, IndexEngine};
use vrtc_observability::{ObservabilityConfig, ObservabilityEngine};
use vrtc_policy::{PolicyConfig, PolicyEngine};
use vrtc_queue::{JobQueueEngine, QueueConfig};
use vrtc_registry::Registry;
use vrtc_research::{ResearchConfig, ResearchPlanner};
use vrtc_retrieval::{RetrievalConfig, RetrievalEngine};
use vrtc_search::{SearchConfig, SearchEngine, SearchProvider};
use vrtc_storage::{StorageConfig, StorageEngine};

const SEARCH_ENGINE_ID: &str = "engine.vrtc.search";
const SEARCH_CAPABILITY_ID: &str = "search.query";
const METASEARCH_ENGINE_ID: &str = "engine.vrtc.metasearch";
const METASEARCH_CAPABILITY_ID: &str = "metasearch.aggregate";
const FRONTIER_ENGINE_ID: &str = "engine.vrtc.frontier";
const FRONTIER_CAPABILITY_ID: &str = "crawl.frontier";
const FETCHER_ENGINE_ID: &str = "engine.vrtc.fetcher";
const FETCHER_CAPABILITY_ID: &str = "http.fetch";
const EXTRACT_ENGINE_ID: &str = "engine.vrtc.extract";
const EXTRACT_CAPABILITY_ID: &str = "content.extract";
const INDEX_ENGINE_ID: &str = "engine.vrtc.index";
const INDEX_CAPABILITY_ID: &str = "index.documents";
const RETRIEVAL_ENGINE_ID: &str = "engine.vrtc.retrieval";
const RETRIEVAL_CAPABILITY_ID: &str = "retrieval.fuse";
const FEEDBACK_ENGINE_ID: &str = "engine.vrtc.feedback";
const FEEDBACK_CAPABILITY_ID: &str = "human.review";
const CRAWLER_ENGINE_ID: &str = "engine.vrtc.crawler";
const CRAWLER_CAPABILITY_ID: &str = "crawl.execute";
const DELTA_ENGINE_ID: &str = "engine.vrtc.delta";
const DELTA_CAPABILITY_ID: &str = "content.delta";
const EVIDENCE_ENGINE_ID: &str = "engine.vrtc.evidence";
const EVIDENCE_CAPABILITY_ID: &str = "evidence.verify";
const STORAGE_ENGINE_ID: &str = "engine.vrtc.storage";
const STORAGE_CAPABILITY_ID: &str = "storage.kv";
const QUEUE_ENGINE_ID: &str = "engine.vrtc.queue";
const QUEUE_CAPABILITY_ID: &str = "queue.jobs";
const RESEARCH_ENGINE_ID: &str = "engine.vrtc.research";
const RESEARCH_CAPABILITY_ID: &str = "research.plan";
const OBSERVABILITY_ENGINE_ID: &str = "engine.vrtc.observability";
const OBSERVABILITY_CAPABILITY_ID: &str = "observability.telemetry";
const POLICY_ENGINE_ID: &str = "engine.vrtc.policy";
const POLICY_CAPABILITY_ID: &str = "policy.evaluate";
const ARCHIVE_ENGINE_ID: &str = "engine.vrtc.archive";
const ARCHIVE_CAPABILITY_ID: &str = "archive.warc";
const BROWSER_ENGINE_ID: &str = "engine.vrtc.browser";
const BROWSER_CAPABILITY_ID: &str = "browser.render";

/// Configuration for the engines owned by the VRTc runtime.
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig {
    pub search: SearchConfig,
    pub frontier: FrontierConfig,
    pub fetch: FetchConfig,
    pub extraction: ExtractionConfig,
    pub index: IndexConfig,
    pub retrieval: RetrievalConfig,
    pub feedback: FeedbackConfig,
    pub crawler: CrawlerConfig,
    pub delta: DeltaConfig,
    pub evidence: EvidenceConfig,
    pub storage: StorageConfig,
    pub storage_path: Option<PathBuf>,
    pub queue: QueueConfig,
    pub research: ResearchConfig,
    pub observability: ObservabilityConfig,
    pub policy: PolicyConfig,
    pub archive: ArchiveConfig,
    pub browser: BrowserRenderConfig,
}

/// Main VRTc runtime façade. The initial runtime is modular and in-process.
pub struct VrtcRuntime {
    registry: Arc<Registry>,
    search: Arc<SearchEngine>,
    frontier: Arc<FrontierEngine>,
    fetcher: Arc<FetchEngine>,
    extraction: Arc<ExtractionEngine>,
    index: Arc<IndexEngine>,
    retrieval: Arc<RetrievalEngine>,
    feedback: Arc<FeedbackEngine>,
    crawler: Arc<CrawlerEngine>,
    delta: Arc<DeltaEngine>,
    evidence: Arc<EvidenceEngine>,
    storage: Arc<StorageEngine>,
    queue: Arc<JobQueueEngine>,
    research: Arc<ResearchPlanner>,
    observability: Arc<ObservabilityEngine>,
    policy: Arc<PolicyEngine>,
    archive: Arc<ArchiveEngine>,
    browser: Arc<BrowserRenderer>,
    registration_lock: Mutex<()>,
}

impl VrtcRuntime {
    /// Initialize the runtime and statically register the built-in Search Engine.
    pub fn new(config: RuntimeConfig) -> Result<Self, VrtCError> {
        let registry = Arc::new(Registry::new());
        let search = Arc::new(SearchEngine::new(config.search)?);
        let frontier = Arc::new(FrontierEngine::new(config.frontier)?);
        let fetcher = Arc::new(FetchEngine::new(config.fetch)?);
        let extraction = Arc::new(ExtractionEngine::new(config.extraction)?);
        let index = Arc::new(IndexEngine::new(config.index)?);
        let retrieval = Arc::new(RetrievalEngine::new(Arc::clone(&index), config.retrieval)?);
        let feedback = Arc::new(FeedbackEngine::new(config.feedback)?);
        let delta = Arc::new(DeltaEngine::new(config.delta)?);
        let evidence = Arc::new(EvidenceEngine::new(config.evidence)?);
        let storage = Arc::new(match config.storage_path.as_deref() {
            Some(path) => StorageEngine::open_redb(path, config.storage)?,
            None => StorageEngine::memory(config.storage)?,
        });
        let queue = Arc::new(JobQueueEngine::new(
            config.queue,
            Some(Arc::clone(&storage)),
        )?);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        queue.recover_expired(now_ms)?;
        let research = Arc::new(ResearchPlanner::new(config.research)?);
        let observability = Arc::new(ObservabilityEngine::new(config.observability)?);
        let policy = Arc::new(PolicyEngine::new(config.policy)?);
        let archive = Arc::new(ArchiveEngine::new(config.archive)?);
        let browser = Arc::new(BrowserRenderer::new(config.browser, Arc::clone(&policy))?);
        let crawler = Arc::new(CrawlerEngine::new(
            config.crawler,
            Arc::clone(&frontier),
            Arc::clone(&fetcher),
            Arc::clone(&extraction),
            Arc::clone(&index),
            Arc::clone(&delta),
        )?);
        registry.register_static_engine(search_engine_descriptor())?;
        registry.activate_engine(SEARCH_ENGINE_ID)?;
        registry.register_static_engine(metasearch_engine_descriptor())?;
        registry.activate_engine(METASEARCH_ENGINE_ID)?;
        registry.register_static_engine(frontier_engine_descriptor())?;
        registry.activate_engine(FRONTIER_ENGINE_ID)?;
        registry.register_static_engine(fetcher_engine_descriptor())?;
        registry.activate_engine(FETCHER_ENGINE_ID)?;
        registry.register_static_engine(extraction_engine_descriptor())?;
        registry.activate_engine(EXTRACT_ENGINE_ID)?;
        registry.register_static_engine(index_engine_descriptor())?;
        registry.activate_engine(INDEX_ENGINE_ID)?;
        registry.register_static_engine(retrieval_engine_descriptor())?;
        registry.activate_engine(RETRIEVAL_ENGINE_ID)?;
        registry.register_static_engine(feedback_engine_descriptor())?;
        registry.activate_engine(FEEDBACK_ENGINE_ID)?;
        registry.register_static_engine(crawler_engine_descriptor())?;
        registry.activate_engine(CRAWLER_ENGINE_ID)?;
        registry.register_static_engine(delta_engine_descriptor())?;
        registry.activate_engine(DELTA_ENGINE_ID)?;
        registry.register_static_engine(evidence_engine_descriptor())?;
        registry.activate_engine(EVIDENCE_ENGINE_ID)?;
        registry.register_static_engine(storage_engine_descriptor())?;
        registry.activate_engine(STORAGE_ENGINE_ID)?;
        registry.register_static_engine(queue_engine_descriptor())?;
        registry.activate_engine(QUEUE_ENGINE_ID)?;
        registry.register_static_engine(research_engine_descriptor())?;
        registry.activate_engine(RESEARCH_ENGINE_ID)?;
        registry.register_static_engine(observability_engine_descriptor())?;
        registry.activate_engine(OBSERVABILITY_ENGINE_ID)?;
        registry.register_static_engine(policy_engine_descriptor())?;
        registry.activate_engine(POLICY_ENGINE_ID)?;
        registry.register_static_engine(archive_engine_descriptor())?;
        registry.activate_engine(ARCHIVE_ENGINE_ID)?;
        registry.register_static_engine(browser_engine_descriptor())?;
        registry.activate_engine(BROWSER_ENGINE_ID)?;

        Ok(Self {
            registry,
            search,
            frontier,
            fetcher,
            extraction,
            index,
            retrieval,
            feedback,
            crawler,
            delta,
            evidence,
            storage,
            queue,
            research,
            observability,
            policy,
            archive,
            browser,
            registration_lock: Mutex::new(()),
        })
    }

    /// Register a typed search adapter and its descriptor as one runtime operation.
    ///
    /// Dynamic registration adds a typed adapter handle, not arbitrary native code.
    pub fn register_search_provider(
        &self,
        descriptor: ProviderDescriptor,
        provider: Arc<dyn SearchProvider>,
        source: RegistrationSource,
    ) -> Result<(), VrtCError> {
        let _guard = self.registration_lock.lock().map_err(|_| {
            internal_error(
                "RUNTIME_REGISTRATION_LOCK_POISONED",
                "Runtime registration is temporarily unavailable",
            )
        })?;

        if descriptor.id != provider.id() {
            return Err(VrtCError::new(
                "PROVIDER_ID_MISMATCH",
                "Provider descriptor ID must match the executable adapter ID",
                ErrorCategory::Validation,
                false,
            )
            .for_component(provider.id()));
        }

        if !descriptor.capabilities.iter().any(|capability| {
            capability.id == SEARCH_CAPABILITY_ID
                && capability
                    .operations
                    .iter()
                    .any(|operation| operation == "search")
        }) {
            return Err(VrtCError::new(
                "SEARCH_CAPABILITY_MISSING",
                "Search provider descriptor must declare the search.query capability and search operation",
                ErrorCategory::Validation,
                false,
            )
            .for_component(&descriptor.id));
        }

        if self
            .search
            .provider_ids()
            .iter()
            .any(|id| id == &descriptor.id)
        {
            return Err(VrtCError::new(
                "DUPLICATE_PROVIDER_ID",
                "A search adapter with this identifier is already registered",
                ErrorCategory::DuplicateId,
                false,
            )
            .for_component(&descriptor.id));
        }

        match source {
            RegistrationSource::Static => {
                self.registry.register_static_provider(descriptor.clone())?
            }
            RegistrationSource::Dynamic => self
                .registry
                .register_dynamic_provider(descriptor.clone())?,
        }
        self.registry.activate_provider(&descriptor.id)?;

        if let Err(error) = self.search.register_provider(provider) {
            let _ = self.registry.disable_provider(&descriptor.id);
            return Err(error);
        }

        Ok(())
    }

    /// Read-only access to the shared component registry.
    #[must_use]
    pub fn registry(&self) -> Arc<Registry> {
        Arc::clone(&self.registry)
    }

    /// Shared Search Engine handle for the HTTP API or embedded callers.
    #[must_use]
    pub fn search_engine(&self) -> Arc<SearchEngine> {
        Arc::clone(&self.search)
    }

    /// Shared native Metasearch handle. It uses the same source catalog and
    /// applies VRTc-owned rank fusion to independent provider result lists.
    #[must_use]
    pub fn metasearch_engine(&self) -> Arc<SearchEngine> {
        Arc::clone(&self.search)
    }

    /// Shared native URL Frontier handle for crawler and fetcher subsystems.
    #[must_use]
    pub fn frontier_engine(&self) -> Arc<FrontierEngine> {
        Arc::clone(&self.frontier)
    }

    /// Shared native HTTP Fetcher handle for crawler and research workflows.
    #[must_use]
    pub fn fetch_engine(&self) -> Arc<FetchEngine> {
        Arc::clone(&self.fetcher)
    }

    /// Shared native HTML Extraction handle for parser and research workflows.
    #[must_use]
    pub fn extraction_engine(&self) -> Arc<ExtractionEngine> {
        Arc::clone(&self.extraction)
    }

    /// Shared native document Index handle for indexing and lexical retrieval.
    #[must_use]
    pub fn index_engine(&self) -> Arc<IndexEngine> {
        Arc::clone(&self.index)
    }

    /// Shared native Retrieval & Reranking handle for hybrid result fusion.
    #[must_use]
    pub fn retrieval_engine(&self) -> Arc<RetrievalEngine> {
        Arc::clone(&self.retrieval)
    }

    /// Shared native Human Feedback and Evaluation handle.
    #[must_use]
    pub fn feedback_engine(&self) -> Arc<FeedbackEngine> {
        Arc::clone(&self.feedback)
    }

    /// Shared native policy-aware Crawler orchestration handle.
    #[must_use]
    pub fn crawler_engine(&self) -> Arc<CrawlerEngine> {
        Arc::clone(&self.crawler)
    }

    /// Shared native content Delta and version-history handle.
    #[must_use]
    pub fn delta_engine(&self) -> Arc<DeltaEngine> {
        Arc::clone(&self.delta)
    }

    /// Shared native evidence provenance and claim-verification handle.
    #[must_use]
    pub fn evidence_engine(&self) -> Arc<EvidenceEngine> {
        Arc::clone(&self.evidence)
    }

    /// Shared native namespaced storage handle.
    #[must_use]
    pub fn storage_engine(&self) -> Arc<StorageEngine> {
        Arc::clone(&self.storage)
    }

    /// Shared native job queue and lease lifecycle handle.
    #[must_use]
    pub fn queue_engine(&self) -> Arc<JobQueueEngine> {
        Arc::clone(&self.queue)
    }

    /// Shared deterministic research planner; execution is handled separately.
    #[must_use]
    pub fn research_planner(&self) -> Arc<ResearchPlanner> {
        Arc::clone(&self.research)
    }

    /// Shared native metrics and operational event telemetry handle.
    #[must_use]
    pub fn observability_engine(&self) -> Arc<ObservabilityEngine> {
        Arc::clone(&self.observability)
    }

    /// Shared native URL, origin, host, and crawl policy decision engine.
    #[must_use]
    pub fn policy_engine(&self) -> Arc<PolicyEngine> {
        Arc::clone(&self.policy)
    }

    /// Shared bounded WARC-compatible web archive engine.
    #[must_use]
    pub fn archive_engine(&self) -> Arc<ArchiveEngine> {
        Arc::clone(&self.archive)
    }

    /// Shared isolated offline Chromium snapshot renderer.
    #[must_use]
    pub fn browser_renderer(&self) -> Arc<BrowserRenderer> {
        Arc::clone(&self.browser)
    }

    /// Execute a search through the runtime-owned Search Engine.
    pub async fn search(&self, request: SearchRequest) -> Result<SearchResponse, VrtCError> {
        self.search.search(request).await
    }
}

fn search_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: SEARCH_ENGINE_ID.to_owned(),
        name: "VRTc Search Engine".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "search".to_owned(),
        capabilities: vec![Capability {
            id: SEARCH_CAPABILITY_ID.to_owned(),
            operations: vec!["search".to_owned()],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::new(),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn metasearch_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: METASEARCH_ENGINE_ID.to_owned(),
        name: "VRTc Native Metasearch & Rank Fusion".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "metasearch".to_owned(),
        capabilities: vec![Capability {
            id: METASEARCH_CAPABILITY_ID.to_owned(),
            operations: vec![
                "fan_out".to_owned(),
                "filter".to_owned(),
                "deduplicate".to_owned(),
                "rank_fusion".to_owned(),
                "preserve_provenance".to_owned(),
            ],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::from([
                ("fusion".to_owned(), "reciprocal-rank-fusion".to_owned()),
                ("rrf_constant".to_owned(), "configurable".to_owned()),
                ("provider_failure".to_owned(), "isolated".to_owned()),
            ]),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn frontier_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: FRONTIER_ENGINE_ID.to_owned(),
        name: "VRTc URL Frontier & Scheduler".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "frontier".to_owned(),
        capabilities: vec![Capability {
            id: FRONTIER_CAPABILITY_ID.to_owned(),
            operations: vec![
                "enqueue".to_owned(),
                "lease".to_owned(),
                "complete".to_owned(),
                "retry".to_owned(),
                "recover".to_owned(),
            ],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::new(),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn fetcher_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: FETCHER_ENGINE_ID.to_owned(),
        name: "VRTc HTTP Fetcher Engine".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "fetcher".to_owned(),
        capabilities: vec![Capability {
            id: FETCHER_CAPABILITY_ID.to_owned(),
            operations: vec!["fetch".to_owned()],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::from([
                ("schemes".to_owned(), "http,https".to_owned()),
                (
                    "network_scope".to_owned(),
                    "public-addresses-only".to_owned(),
                ),
                ("response_limit".to_owned(), "configurable".to_owned()),
            ]),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn extraction_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: EXTRACT_ENGINE_ID.to_owned(),
        name: "VRTc Native Content Extraction Engine".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "extractor".to_owned(),
        capabilities: vec![Capability {
            id: EXTRACT_CAPABILITY_ID.to_owned(),
            operations: vec![
                "extract_html".to_owned(),
                "extract_metadata".to_owned(),
                "extract_links".to_owned(),
            ],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::from([
                ("input".to_owned(), "html".to_owned()),
                (
                    "output".to_owned(),
                    "text,markdown,metadata,links,provenance".to_owned(),
                ),
            ]),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn index_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: INDEX_ENGINE_ID.to_owned(),
        name: "VRTc Native Document Index".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "index".to_owned(),
        capabilities: vec![Capability {
            id: INDEX_CAPABILITY_ID.to_owned(),
            operations: vec![
                "upsert_batch".to_owned(),
                "delete".to_owned(),
                "search".to_owned(),
                "stats".to_owned(),
            ],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::from([
                (
                    "storage".to_owned(),
                    "in-memory-or-local-directory".to_owned(),
                ),
                ("retrieval".to_owned(), "lexical-bm25".to_owned()),
            ]),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn retrieval_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: RETRIEVAL_ENGINE_ID.to_owned(),
        name: "VRTc Native Hybrid Retrieval & Reranking".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "retrieval".to_owned(),
        capabilities: vec![Capability {
            id: RETRIEVAL_CAPABILITY_ID.to_owned(),
            operations: vec![
                "retrieve".to_owned(),
                "fuse".to_owned(),
                "rerank".to_owned(),
            ],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::from([
                ("fusion".to_owned(), "weighted-rrf".to_owned()),
                ("lexical_backend".to_owned(), "vrtc-index".to_owned()),
                (
                    "semantic_backend".to_owned(),
                    "optional-candidate-input".to_owned(),
                ),
            ]),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn feedback_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: FEEDBACK_ENGINE_ID.to_owned(),
        name: "VRTc Human Feedback & Search Evaluation".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "human-feedback".to_owned(),
        capabilities: vec![Capability {
            id: FEEDBACK_CAPABILITY_ID.to_owned(),
            operations: vec![
                "create_review_task".to_owned(),
                "list_review_tasks".to_owned(),
                "submit_judgment".to_owned(),
                "adjudicate".to_owned(),
                "evaluate_ranking".to_owned(),
            ],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::from([
                ("storage".to_owned(), "in-memory-initial".to_owned()),
                ("labels".to_owned(), "graded-0-to-3".to_owned()),
                ("metrics".to_owned(), "precision-recall-mrr-ndcg".to_owned()),
            ]),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn crawler_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: CRAWLER_ENGINE_ID.to_owned(),
        name: "VRTc Native Policy-Aware Crawler".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "crawler".to_owned(),
        capabilities: vec![Capability {
            id: CRAWLER_CAPABILITY_ID.to_owned(),
            operations: vec![
                "crawl_seeds".to_owned(),
                "discover_links".to_owned(),
                "enforce_robots".to_owned(),
                "extract_and_index".to_owned(),
                "report_run".to_owned(),
            ],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::from([
                ("execution".to_owned(), "sequential-bounded-run".to_owned()),
                ("scope".to_owned(), "same-origin-by-default".to_owned()),
                (
                    "robots".to_owned(),
                    "rfc9309-conservative-policy".to_owned(),
                ),
                ("network".to_owned(), "public-addresses-only".to_owned()),
            ]),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn delta_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: DELTA_ENGINE_ID.to_owned(),
        name: "VRTc Native Delta & Versioning Engine".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "delta".to_owned(),
        capabilities: vec![Capability {
            id: DELTA_CAPABILITY_ID.to_owned(),
            operations: vec![
                "record_snapshot".to_owned(),
                "detect_change".to_owned(),
                "compare_versions".to_owned(),
                "read_history".to_owned(),
                "report_stats".to_owned(),
            ],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::from([
                ("fingerprint".to_owned(), "sha-256".to_owned()),
                ("diff".to_owned(), "bounded-line-diff".to_owned()),
                ("storage".to_owned(), "bounded-in-memory-history".to_owned()),
            ]),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn evidence_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: EVIDENCE_ENGINE_ID.to_owned(),
        name: "VRTc Native Evidence & Verification Engine".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "evidence".to_owned(),
        capabilities: vec![Capability {
            id: EVIDENCE_CAPABILITY_ID.to_owned(),
            operations: vec![
                "create_claim".to_owned(),
                "attach_evidence".to_owned(),
                "assess_evidence".to_owned(),
                "verify_claim".to_owned(),
                "read_provenance".to_owned(),
            ],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::from([
                ("assessment".to_owned(), "explicit-not-inferred".to_owned()),
                (
                    "provenance".to_owned(),
                    "source-url-content-hash-retrieval-time".to_owned(),
                ),
                ("storage".to_owned(), "bounded-in-memory-ledger".to_owned()),
            ]),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn storage_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: STORAGE_ENGINE_ID.to_owned(),
        name: "VRTc Native Storage & Database Engine".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "storage".to_owned(),
        capabilities: vec![Capability {
            id: STORAGE_CAPABILITY_ID.to_owned(),
            operations: vec![
                "get".to_owned(),
                "put".to_owned(),
                "delete".to_owned(),
                "apply_batch".to_owned(),
                "scan_namespace".to_owned(),
            ],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::from([
                ("backends".to_owned(), "memory,redb".to_owned()),
                ("batch".to_owned(), "atomic".to_owned()),
                ("namespace".to_owned(), "required".to_owned()),
            ]),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn queue_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: QUEUE_ENGINE_ID.to_owned(),
        name: "VRTc Native Job Queue & Processing Engine".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "queue".to_owned(),
        capabilities: vec![Capability {
            id: QUEUE_CAPABILITY_ID.to_owned(),
            operations: vec![
                "submit".to_owned(),
                "lease".to_owned(),
                "ack".to_owned(),
                "retry".to_owned(),
                "recover_expired".to_owned(),
                "dead_letters".to_owned(),
            ],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::from([
                ("coordination".to_owned(), "single-runtime".to_owned()),
                (
                    "persistence".to_owned(),
                    "optional-storage-engine".to_owned(),
                ),
                ("delivery".to_owned(), "at-least-once".to_owned()),
            ]),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn research_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: RESEARCH_ENGINE_ID.to_owned(),
        name: "VRTc Native Research Planner & Orchestration".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "research-orchestrator".to_owned(),
        capabilities: vec![
            Capability {
                id: RESEARCH_CAPABILITY_ID.to_owned(),
                operations: vec![
                    "plan".to_owned(),
                    "validate_graph".to_owned(),
                    "topological_order".to_owned(),
                ],
                input_contract_version: "1".to_owned(),
                output_contract_version: "1".to_owned(),
                constraints: BTreeMap::from([
                    ("execution".to_owned(), "planning-only".to_owned()),
                    ("graph".to_owned(), "bounded-dag".to_owned()),
                    ("parallelism".to_owned(), "dependency-groups".to_owned()),
                ]),
                availability: CapabilityAvailability::Available,
            },
            Capability {
                id: "research.synthesize".to_owned(),
                operations: vec!["synthesize".to_owned()],
                input_contract_version: "1".to_owned(),
                output_contract_version: "1".to_owned(),
                constraints: BTreeMap::from([(
                    "execution".to_owned(),
                    "external-synthesis-worker".to_owned(),
                )]),
                availability: CapabilityAvailability::Available,
            },
        ],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn observability_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: OBSERVABILITY_ENGINE_ID.to_owned(),
        name: "VRTc Native Monitoring & Observability Engine".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "observability".to_owned(),
        capabilities: vec![Capability {
            id: OBSERVABILITY_CAPABILITY_ID.to_owned(),
            operations: vec![
                "increment_counter".to_owned(),
                "set_gauge".to_owned(),
                "observe_histogram".to_owned(),
                "record_event".to_owned(),
                "snapshot".to_owned(),
            ],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::from([
                ("storage".to_owned(), "bounded-in-memory".to_owned()),
                (
                    "telemetry".to_owned(),
                    "metrics-and-operational-events".to_owned(),
                ),
                (
                    "payload_policy".to_owned(),
                    "no-raw-request-content".to_owned(),
                ),
            ]),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn policy_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: POLICY_ENGINE_ID.to_owned(),
        name: "VRTc Native Security & Retrieval Policy Engine".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "policy".to_owned(),
        capabilities: vec![Capability {
            id: POLICY_CAPABILITY_ID.to_owned(),
            operations: vec![
                "evaluate_url".to_owned(),
                "enforce_host_rules".to_owned(),
                "enforce_origin_scope".to_owned(),
                "enforce_crawl_depth".to_owned(),
                "require_robots_approval".to_owned(),
            ],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::from([
                ("schemes".to_owned(), "http,https-configurable".to_owned()),
                (
                    "private_addresses".to_owned(),
                    "blocked-by-default".to_owned(),
                ),
                (
                    "crawl_scope".to_owned(),
                    "same-origin-by-default".to_owned(),
                ),
                (
                    "robots".to_owned(),
                    "approval-required-for-crawl".to_owned(),
                ),
            ]),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn archive_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: ARCHIVE_ENGINE_ID.to_owned(),
        name: "VRTc Native WARC Web Archive Engine".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "web-archive".to_owned(),
        capabilities: vec![Capability {
            id: ARCHIVE_CAPABILITY_ID.to_owned(),
            operations: vec![
                "capture_response".to_owned(),
                "lookup_record".to_owned(),
                "lookup_target".to_owned(),
                "export_warc".to_owned(),
                "archive_stats".to_owned(),
            ],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::from([
                ("format".to_owned(), "warc-1.1-response".to_owned()),
                ("retention".to_owned(), "bounded-in-memory".to_owned()),
                ("payload".to_owned(), "response-body".to_owned()),
            ]),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn browser_engine_descriptor() -> EngineDescriptor {
    EngineDescriptor {
        id: BROWSER_ENGINE_ID.to_owned(),
        name: "VRTc Native Offline Browser Rendering Engine".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        contract_version: "1".to_owned(),
        kind: "browser-renderer".to_owned(),
        capabilities: vec![Capability {
            id: BROWSER_CAPABILITY_ID.to_owned(),
            operations: vec![
                "render_fetched_html".to_owned(),
                "extract_rendered_dom".to_owned(),
                "read_document_title".to_owned(),
            ],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::from([
                ("network".to_owned(), "offline-snapshot-only".to_owned()),
                ("javascript".to_owned(), "disabled".to_owned()),
                ("policy".to_owned(), "native-policy-required".to_owned()),
                ("input".to_owned(), "bounded-html".to_owned()),
            ]),
            availability: CapabilityAvailability::Available,
        }],
        lifecycle_state: LifecycleState::Discovered,
        registration_source: RegistrationSource::Static,
        metadata: BTreeMap::new(),
    }
}

fn internal_error(code: &str, message: &str) -> VrtCError {
    VrtCError::new(code, message, ErrorCategory::Internal, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::BTreeMap;
    use vrtc_core::{ConnectionMode, SearchResult};

    struct StubProvider {
        id: &'static str,
    }

    #[async_trait]
    impl SearchProvider for StubProvider {
        fn id(&self) -> &str {
            self.id
        }

        async fn search(&self, _request: SearchRequest) -> Result<Vec<SearchResult>, VrtCError> {
            Ok(vec![SearchResult {
                title: "Runtime integration".to_owned(),
                url: "https://example.org/runtime".to_owned(),
                snippet: None,
                score: Some(1.0),
                provider_id: self.id.to_owned(),
                published_at: None,
                language: None,
                metadata: BTreeMap::new(),
            }])
        }
    }

    fn provider_descriptor(id: &str) -> ProviderDescriptor {
        ProviderDescriptor {
            id: id.to_owned(),
            name: "Runtime test provider".to_owned(),
            version: "0.1.0".to_owned(),
            contract_version: "1".to_owned(),
            capabilities: vec![Capability {
                id: SEARCH_CAPABILITY_ID.to_owned(),
                operations: vec!["search".to_owned()],
                input_contract_version: "1".to_owned(),
                output_contract_version: "1".to_owned(),
                constraints: BTreeMap::new(),
                availability: CapabilityAvailability::Available,
            }],
            connection_mode: ConnectionMode::Local,
            lifecycle_state: LifecycleState::Discovered,
            registration_source: RegistrationSource::Dynamic,
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn built_in_browser_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(BROWSER_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "browser-renderer");
        assert_eq!(engine.capabilities[0].id, BROWSER_CAPABILITY_ID);
    }

    #[test]
    fn built_in_archive_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(ARCHIVE_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "web-archive");
        assert_eq!(engine.capabilities[0].id, ARCHIVE_CAPABILITY_ID);
        assert_eq!(
            runtime.archive_engine().stats().unwrap().records_retained,
            0
        );
    }

    #[test]
    fn built_in_policy_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(POLICY_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "policy");
        assert_eq!(engine.capabilities[0].id, POLICY_CAPABILITY_ID);
    }

    #[test]
    fn built_in_observability_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(OBSERVABILITY_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "observability");
        assert_eq!(engine.capabilities[0].id, OBSERVABILITY_CAPABILITY_ID);
    }

    #[test]
    fn built_in_research_planner_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(RESEARCH_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "research-orchestrator");
        assert_eq!(engine.capabilities[0].id, RESEARCH_CAPABILITY_ID);
    }

    #[test]
    fn built_in_queue_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(QUEUE_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "queue");
        assert_eq!(engine.capabilities[0].id, QUEUE_CAPABILITY_ID);
    }

    #[test]
    fn built_in_storage_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(STORAGE_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "storage");
        assert_eq!(engine.capabilities[0].id, STORAGE_CAPABILITY_ID);
    }

    #[test]
    fn built_in_evidence_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(EVIDENCE_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "evidence");
        assert_eq!(engine.capabilities[0].id, EVIDENCE_CAPABILITY_ID);
    }

    #[test]
    fn built_in_delta_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(DELTA_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "delta");
        assert_eq!(engine.capabilities[0].id, DELTA_CAPABILITY_ID);
    }

    #[test]
    fn built_in_crawler_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(CRAWLER_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "crawler");
        assert_eq!(engine.capabilities[0].id, CRAWLER_CAPABILITY_ID);
    }

    #[test]
    fn built_in_metasearch_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(METASEARCH_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "metasearch");
        assert_eq!(engine.capabilities[0].id, METASEARCH_CAPABILITY_ID);
    }

    #[test]
    fn built_in_feedback_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(FEEDBACK_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "human-feedback");
        assert_eq!(engine.capabilities[0].id, FEEDBACK_CAPABILITY_ID);
        assert!(engine.capabilities[0]
            .operations
            .iter()
            .any(|operation| operation == "list_review_tasks"));
    }

    #[tokio::test]
    async fn registered_provider_is_visible_and_executable_through_runtime() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        runtime
            .register_search_provider(
                provider_descriptor("provider.runtime.stub"),
                Arc::new(StubProvider {
                    id: "provider.runtime.stub",
                }),
                RegistrationSource::Dynamic,
            )
            .unwrap();

        let registered = runtime
            .registry()
            .provider("provider.runtime.stub")
            .unwrap();
        assert_eq!(registered.lifecycle_state, LifecycleState::Ready);
        assert_eq!(
            runtime
                .registry()
                .providers_for_capability(SEARCH_CAPABILITY_ID)
                .len(),
            1
        );

        let response = runtime.search(SearchRequest::new("runtime")).await.unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].provider_id, "provider.runtime.stub");
    }

    #[test]
    fn descriptor_and_adapter_ids_must_match() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let error = runtime
            .register_search_provider(
                provider_descriptor("provider.descriptor"),
                Arc::new(StubProvider {
                    id: "provider.adapter",
                }),
                RegistrationSource::Static,
            )
            .unwrap_err();

        assert_eq!(error.code, "PROVIDER_ID_MISMATCH");
        assert!(runtime.registry().provider("provider.descriptor").is_none());
    }

    #[test]
    fn built_in_retrieval_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(RETRIEVAL_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "retrieval");
        assert_eq!(engine.capabilities[0].id, RETRIEVAL_CAPABILITY_ID);
    }

    #[test]
    fn built_in_index_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(INDEX_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "index");
        assert_eq!(engine.capabilities[0].id, INDEX_CAPABILITY_ID);
    }

    #[test]
    fn built_in_extraction_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(EXTRACT_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "extractor");
        assert_eq!(engine.capabilities[0].id, EXTRACT_CAPABILITY_ID);
    }

    #[test]
    fn built_in_fetcher_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(FETCHER_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "fetcher");
        assert_eq!(engine.capabilities[0].id, FETCHER_CAPABILITY_ID);
    }

    #[test]
    fn built_in_frontier_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(FRONTIER_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.kind, "frontier");
        assert_eq!(engine.capabilities[0].id, FRONTIER_CAPABILITY_ID);
    }

    #[test]
    fn built_in_search_engine_is_registered_and_ready() {
        let runtime = VrtcRuntime::new(RuntimeConfig::default()).unwrap();
        let engine = runtime.registry().engine(SEARCH_ENGINE_ID).unwrap();
        assert_eq!(engine.lifecycle_state, LifecycleState::Ready);
        assert_eq!(engine.registration_source, RegistrationSource::Static);
    }
}
