//! VRTc runtime façade joining component registration to executable engines.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use vrtc_core::{
    Capability, CapabilityAvailability, EngineDescriptor, ErrorCategory, LifecycleState,
    ProviderDescriptor, RegistrationSource, SearchRequest, SearchResponse, VrtCError,
};
use vrtc_fetcher::{FetchConfig, FetchEngine};
use vrtc_frontier::{FrontierConfig, FrontierEngine};
use vrtc_registry::Registry;
use vrtc_search::{SearchConfig, SearchEngine, SearchProvider};

const SEARCH_ENGINE_ID: &str = "engine.vrtc.search";
const SEARCH_CAPABILITY_ID: &str = "search.query";
const FRONTIER_ENGINE_ID: &str = "engine.vrtc.frontier";
const FRONTIER_CAPABILITY_ID: &str = "crawl.frontier";
const FETCHER_ENGINE_ID: &str = "engine.vrtc.fetcher";
const FETCHER_CAPABILITY_ID: &str = "http.fetch";

/// Runtime configuration for the initial Search Engine subsystem.
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig {
    pub search: SearchConfig,
    pub frontier: FrontierConfig,
    pub fetch: FetchConfig,
}

/// Main VRTc runtime façade. The initial runtime is modular and in-process.
pub struct VrtcRuntime {
    registry: Arc<Registry>,
    search: Arc<SearchEngine>,
    frontier: Arc<FrontierEngine>,
    fetcher: Arc<FetchEngine>,
    registration_lock: Mutex<()>,
}

impl VrtcRuntime {
    /// Initialize the runtime and statically register the built-in Search Engine.
    pub fn new(config: RuntimeConfig) -> Result<Self, VrtCError> {
        let registry = Arc::new(Registry::new());
        let search = Arc::new(SearchEngine::new(config.search)?);
        let frontier = Arc::new(FrontierEngine::new(config.frontier)?);
        let fetcher = Arc::new(FetchEngine::new(config.fetch)?);
        registry.register_static_engine(search_engine_descriptor())?;
        registry.activate_engine(SEARCH_ENGINE_ID)?;
        registry.register_static_engine(frontier_engine_descriptor())?;
        registry.activate_engine(FRONTIER_ENGINE_ID)?;
        registry.register_static_engine(fetcher_engine_descriptor())?;
        registry.activate_engine(FETCHER_ENGINE_ID)?;

        Ok(Self {
            registry,
            search,
            frontier,
            fetcher,
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
