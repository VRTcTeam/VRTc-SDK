//! VRTc runtime façade joining component registration to executable engines.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use vrtc_core::{
    Capability, CapabilityAvailability, EngineDescriptor, ErrorCategory, LifecycleState,
    ProviderDescriptor, RegistrationSource, SearchRequest, SearchResponse, VrtCError,
};
use vrtc_extract::{ExtractionConfig, ExtractionEngine};
use vrtc_feedback::{FeedbackConfig, FeedbackEngine};
use vrtc_fetcher::{FetchConfig, FetchEngine};
use vrtc_frontier::{FrontierConfig, FrontierEngine};
use vrtc_index::{IndexConfig, IndexEngine};
use vrtc_registry::Registry;
use vrtc_retrieval::{RetrievalConfig, RetrievalEngine};
use vrtc_search::{SearchConfig, SearchEngine, SearchProvider};

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

        Ok(Self {
            registry,
            search,
            frontier,
            fetcher,
            extraction,
            index,
            retrieval,
            feedback,
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
