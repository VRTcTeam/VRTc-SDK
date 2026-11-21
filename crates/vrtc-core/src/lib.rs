//! Shared, provider-agnostic contracts for the VRTc runtime.
//!
//! These types describe components and their capabilities. They do not load
//! or execute provider code and must never contain credential values.

use std::collections::BTreeMap;

/// Stable identifier for an engine, provider, or capability.
pub type ComponentId = String;

/// Registration path used to add a component to the runtime catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationSource {
    Static,
    Dynamic,
}

/// Lifecycle state is intentionally separate from health status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleState {
    Discovered,
    Validated,
    Registered,
    Initializing,
    Ready,
    Degraded,
    Disabled,
    ShuttingDown,
    Stopped,
    Failed,
}

/// Last known operational health of a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

/// Availability of one declared capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityAvailability {
    Available,
    Degraded,
    Unavailable,
}

/// Machine-readable capability declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capability {
    pub id: ComponentId,
    pub operations: Vec<String>,
    pub input_contract_version: String,
    pub output_contract_version: String,
    pub constraints: BTreeMap<String, String>,
    pub availability: CapabilityAvailability,
}

/// Description of a VRTc engine implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineDescriptor {
    pub id: ComponentId,
    pub name: String,
    pub version: String,
    pub contract_version: String,
    pub kind: String,
    pub capabilities: Vec<Capability>,
    pub lifecycle_state: LifecycleState,
    pub registration_source: RegistrationSource,
    pub metadata: BTreeMap<String, String>,
}

/// Connection mode declared by a provider adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionMode {
    Local,
    RemoteApi,
    Embedded,
    Other(String),
}

/// Description of a local or external provider adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub id: ComponentId,
    pub name: String,
    pub version: String,
    pub contract_version: String,
    pub capabilities: Vec<Capability>,
    pub connection_mode: ConnectionMode,
    pub lifecycle_state: LifecycleState,
    pub registration_source: RegistrationSource,
    pub metadata: BTreeMap<String, String>,
}

/// Stable error categories shared by runtime and language clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    Validation,
    ContractIncompatible,
    DuplicateId,
    NotFound,
    NotRegistered,
    LifecycleInvalid,
    Disabled,
    PolicyDenied,
    HealthCheckFailed,
    Timeout,
    Cancelled,
    ResourceExhausted,
    Internal,
}

/// Structured error. Details must be safe and must not contain secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VrtCError {
    pub code: String,
    pub message: String,
    pub category: ErrorCategory,
    pub retryable: bool,
    pub component_id: Option<ComponentId>,
    pub details: BTreeMap<String, String>,
}

impl VrtCError {
    #[must_use]
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        category: ErrorCategory,
        retryable: bool,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            category,
            retryable,
            component_id: None,
            details: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn for_component(mut self, component_id: impl Into<ComponentId>) -> Self {
        self.component_id = Some(component_id.into());
        self
    }
}


/// Canonical request shared by the VRTc Search Engine and provider adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    pub query: String,
    /// Zero asks the engine to apply its configured default.
    pub limit: usize,
    pub language: Option<String>,
    pub domains: Vec<String>,
    /// RFC 3339 timestamps; lexical comparison is valid for normalized UTC values.
    pub published_after: Option<String>,
    pub published_before: Option<String>,
}

impl SearchRequest {
    #[must_use]
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            limit: 0,
            language: None,
            domains: Vec::new(),
            published_after: None,
            published_before: None,
        }
    }
}

/// One normalized search result with provider provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: Option<String>,
    pub score: Option<f64>,
    pub provider_id: ComponentId,
    pub published_at: Option<String>,
    pub language: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

/// Safe, structured failure reported by an individual search provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSearchError {
    pub provider_id: ComponentId,
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

/// Aggregated response returned by the VRTc Search Engine.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub providers_queried: Vec<ComponentId>,
    pub provider_errors: Vec<ProviderSearchError>,
}
