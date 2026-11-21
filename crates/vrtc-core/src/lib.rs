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
