//! Hybrid registry for VRTc engines and provider adapters.
//!
//! Dynamic registration adds validated metadata to the catalog. It does not
//! load native libraries, execute arbitrary code, or bypass runtime policy.

use std::collections::HashMap;
use std::sync::RwLock;

use vrtc_core::{
    CapabilityAvailability, EngineDescriptor, ErrorCategory, LifecycleState,
    ProviderDescriptor, RegistrationSource, VrtCError,
};

/// Thread-safe catalog with separate engine and provider namespaces.
#[derive(Debug, Default)]
pub struct Registry {
    engines: RwLock<HashMap<String, EngineDescriptor>>,
    providers: RwLock<HashMap<String, ProviderDescriptor>>,
}

impl Registry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_static_engine(
        &self,
        mut descriptor: EngineDescriptor,
    ) -> Result<(), VrtCError> {
        descriptor.registration_source = RegistrationSource::Static;
        self.register_engine(descriptor)
    }

    pub fn register_dynamic_engine(
        &self,
        mut descriptor: EngineDescriptor,
    ) -> Result<(), VrtCError> {
        descriptor.registration_source = RegistrationSource::Dynamic;
        self.register_engine(descriptor)
    }

    fn register_engine(&self, mut descriptor: EngineDescriptor) -> Result<(), VrtCError> {
        validate_identity(&descriptor.id, "engine.")?;
        validate_common_fields(
            &descriptor.id,
            &descriptor.name,
            &descriptor.version,
            &descriptor.contract_version,
        )?;
        validate_capabilities(&descriptor.id, &descriptor.capabilities)?;
        descriptor.lifecycle_state = LifecycleState::Registered;

        let mut engines = self.engines.write().map_err(|_| {
            internal_error("ENGINE_REGISTRY_LOCK_POISONED", "Engine registry lock is unavailable")
        })?;
        if engines.contains_key(&descriptor.id) {
            return Err(duplicate_error(&descriptor.id));
        }
        engines.insert(descriptor.id.clone(), descriptor);
        Ok(())
    }

    pub fn register_static_provider(
        &self,
        mut descriptor: ProviderDescriptor,
    ) -> Result<(), VrtCError> {
        descriptor.registration_source = RegistrationSource::Static;
        self.register_provider(descriptor)
    }

    pub fn register_dynamic_provider(
        &self,
        mut descriptor: ProviderDescriptor,
    ) -> Result<(), VrtCError> {
        descriptor.registration_source = RegistrationSource::Dynamic;
        self.register_provider(descriptor)
    }

    fn register_provider(&self, mut descriptor: ProviderDescriptor) -> Result<(), VrtCError> {
        validate_identity(&descriptor.id, "provider.")?;
        validate_common_fields(
            &descriptor.id,
            &descriptor.name,
            &descriptor.version,
            &descriptor.contract_version,
        )?;
        validate_capabilities(&descriptor.id, &descriptor.capabilities)?;
        descriptor.lifecycle_state = LifecycleState::Registered;

        let mut providers = self.providers.write().map_err(|_| {
            internal_error("PROVIDER_REGISTRY_LOCK_POISONED", "Provider registry lock is unavailable")
        })?;
        if providers.contains_key(&descriptor.id) {
            return Err(duplicate_error(&descriptor.id));
        }
        providers.insert(descriptor.id.clone(), descriptor);
        Ok(())
    }

    pub fn activate_engine(&self, id: &str) -> Result<(), VrtCError> {
        let mut engines = self.engines.write().map_err(|_| {
            internal_error("ENGINE_REGISTRY_LOCK_POISONED", "Engine registry lock is unavailable")
        })?;
        let descriptor = engines
            .get_mut(id)
            .ok_or_else(|| not_found_error(id))?;
        match descriptor.lifecycle_state {
            LifecycleState::Registered | LifecycleState::Disabled | LifecycleState::Degraded => {
                descriptor.lifecycle_state = LifecycleState::Ready;
                Ok(())
            }
            state => Err(lifecycle_error(id, state, LifecycleState::Ready)),
        }
    }

    pub fn activate_provider(&self, id: &str) -> Result<(), VrtCError> {
        let mut providers = self.providers.write().map_err(|_| {
            internal_error("PROVIDER_REGISTRY_LOCK_POISONED", "Provider registry lock is unavailable")
        })?;
        let descriptor = providers
            .get_mut(id)
            .ok_or_else(|| not_found_error(id))?;
        match descriptor.lifecycle_state {
            LifecycleState::Registered | LifecycleState::Disabled | LifecycleState::Degraded => {
                descriptor.lifecycle_state = LifecycleState::Ready;
                Ok(())
            }
            state => Err(lifecycle_error(id, state, LifecycleState::Ready)),
        }
    }

    pub fn disable_engine(&self, id: &str) -> Result<(), VrtCError> {
        let mut engines = self.engines.write().map_err(|_| {
            internal_error("ENGINE_REGISTRY_LOCK_POISONED", "Engine registry lock is unavailable")
        })?;
        let descriptor = engines
            .get_mut(id)
            .ok_or_else(|| not_found_error(id))?;
        if matches!(descriptor.lifecycle_state, LifecycleState::Ready | LifecycleState::Degraded) {
            descriptor.lifecycle_state = LifecycleState::Disabled;
            Ok(())
        } else {
            Err(lifecycle_error(id, descriptor.lifecycle_state, LifecycleState::Disabled))
        }
    }

    pub fn disable_provider(&self, id: &str) -> Result<(), VrtCError> {
        let mut providers = self.providers.write().map_err(|_| {
            internal_error("PROVIDER_REGISTRY_LOCK_POISONED", "Provider registry lock is unavailable")
        })?;
        let descriptor = providers
            .get_mut(id)
            .ok_or_else(|| not_found_error(id))?;
        if matches!(descriptor.lifecycle_state, LifecycleState::Ready | LifecycleState::Degraded) {
            descriptor.lifecycle_state = LifecycleState::Disabled;
            Ok(())
        } else {
            Err(lifecycle_error(id, descriptor.lifecycle_state, LifecycleState::Disabled))
        }
    }

    #[must_use]
    pub fn engine(&self, id: &str) -> Option<EngineDescriptor> {
        self.engines.read().ok()?.get(id).cloned()
    }

    #[must_use]
    pub fn provider(&self, id: &str) -> Option<ProviderDescriptor> {
        self.providers.read().ok()?.get(id).cloned()
    }

    #[must_use]
    pub fn engines_for_capability(&self, capability_id: &str) -> Vec<EngineDescriptor> {
        let Ok(engines) = self.engines.read() else {
            return Vec::new();
        };
        engines
            .values()
            .filter(|engine| {
                matches!(engine.lifecycle_state, LifecycleState::Ready | LifecycleState::Degraded)
                    && engine.capabilities.iter().any(|capability| {
                        capability.id == capability_id
                            && capability.availability != CapabilityAvailability::Unavailable
                    })
            })
            .cloned()
            .collect()
    }

    #[must_use]
    pub fn providers_for_capability(&self, capability_id: &str) -> Vec<ProviderDescriptor> {
        let Ok(providers) = self.providers.read() else {
            return Vec::new();
        };
        providers
            .values()
            .filter(|provider| {
                matches!(provider.lifecycle_state, LifecycleState::Ready | LifecycleState::Degraded)
                    && provider.capabilities.iter().any(|capability| {
                        capability.id == capability_id
                            && capability.availability != CapabilityAvailability::Unavailable
                    })
            })
            .cloned()
            .collect()
    }
}

fn validate_identity(id: &str, prefix: &str) -> Result<(), VrtCError> {
    if !id.starts_with(prefix) || id.len() <= prefix.len() || id.chars().any(char::is_whitespace) {
        return Err(VrtCError::new(
            "INVALID_COMPONENT_ID",
            format!("Component identifier must start with {prefix} and contain no whitespace"),
            ErrorCategory::Validation,
            false,
        ));
    }
    Ok(())
}

fn validate_common_fields(
    id: &str,
    name: &str,
    version: &str,
    contract_version: &str,
) -> Result<(), VrtCError> {
    if name.trim().is_empty() || version.trim().is_empty() || contract_version.trim().is_empty() {
        return Err(VrtCError::new(
            "MISSING_REQUIRED_FIELD",
            "Component name, version, and contract version are required",
            ErrorCategory::Validation,
            false,
        )
        .for_component(id));
    }
    Ok(())
}

fn validate_capabilities(
    component_id: &str,
    capabilities: &[vrtc_core::Capability],
) -> Result<(), VrtCError> {
    for capability in capabilities {
        if capability.id.trim().is_empty()
            || capability.input_contract_version.trim().is_empty()
            || capability.output_contract_version.trim().is_empty()
            || capability.operations.iter().any(|operation| operation.trim().is_empty())
        {
            return Err(VrtCError::new(
                "INVALID_CAPABILITY",
                "Capability identifiers, contract versions, and operations must be non-empty",
                ErrorCategory::Validation,
                false,
            )
            .for_component(component_id));
        }
    }
    Ok(())
}

fn duplicate_error(id: &str) -> VrtCError {
    VrtCError::new(
        "DUPLICATE_COMPONENT_ID",
        "A component with this identifier is already registered",
        ErrorCategory::DuplicateId,
        false,
    )
    .for_component(id)
}

fn not_found_error(id: &str) -> VrtCError {
    VrtCError::new(
        "COMPONENT_NOT_FOUND",
        "The requested component is not registered",
        ErrorCategory::NotFound,
        false,
    )
    .for_component(id)
}

fn lifecycle_error(
    id: &str,
    current: LifecycleState,
    requested: LifecycleState,
) -> VrtCError {
    VrtCError::new(
        "INVALID_LIFECYCLE_TRANSITION",
        format!("Cannot transition component from {current:?} to {requested:?}"),
        ErrorCategory::LifecycleInvalid,
        false,
    )
    .for_component(id)
}

fn internal_error(code: &str, message: &str) -> VrtCError {
    VrtCError::new(code, message, ErrorCategory::Internal, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use vrtc_core::{
        Capability, ConnectionMode, EngineDescriptor, ProviderDescriptor,
    };

    fn capability() -> Capability {
        Capability {
            id: "search.query".to_owned(),
            operations: vec!["search".to_owned()],
            input_contract_version: "1".to_owned(),
            output_contract_version: "1".to_owned(),
            constraints: BTreeMap::new(),
            availability: CapabilityAvailability::Available,
        }
    }

    fn engine(id: &str) -> EngineDescriptor {
        EngineDescriptor {
            id: id.to_owned(),
            name: "Search Engine".to_owned(),
            version: "0.1.0".to_owned(),
            contract_version: "1".to_owned(),
            kind: "search".to_owned(),
            capabilities: vec![capability()],
            lifecycle_state: LifecycleState::Discovered,
            registration_source: RegistrationSource::Static,
            metadata: BTreeMap::new(),
        }
    }

    fn provider(id: &str) -> ProviderDescriptor {
        ProviderDescriptor {
            id: id.to_owned(),
            name: "Local Search Provider".to_owned(),
            version: "0.1.0".to_owned(),
            contract_version: "1".to_owned(),
            capabilities: vec![capability()],
            connection_mode: ConnectionMode::Local,
            lifecycle_state: LifecycleState::Discovered,
            registration_source: RegistrationSource::Static,
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn dynamic_engine_is_registered_and_discoverable_after_activation() {
        let registry = Registry::new();
        registry.register_dynamic_engine(engine("engine.vrtc.search")).unwrap();
        assert_eq!(
            registry.engine("engine.vrtc.search").unwrap().registration_source,
            RegistrationSource::Dynamic
        );
        assert!(registry.engines_for_capability("search.query").is_empty());
        registry.activate_engine("engine.vrtc.search").unwrap();
        assert_eq!(registry.engines_for_capability("search.query").len(), 1);
    }

    #[test]
    fn duplicate_engine_identifier_is_rejected() {
        let registry = Registry::new();
        registry.register_static_engine(engine("engine.vrtc.search")).unwrap();
        let error = registry.register_static_engine(engine("engine.vrtc.search")).unwrap_err();
        assert_eq!(error.category, ErrorCategory::DuplicateId);
    }

    #[test]
    fn provider_registration_and_disable_are_isolated() {
        let registry = Registry::new();
        registry.register_static_provider(provider("provider.local.demo")).unwrap();
        registry.activate_provider("provider.local.demo").unwrap();
        assert_eq!(registry.providers_for_capability("search.query").len(), 1);
        registry.disable_provider("provider.local.demo").unwrap();
        assert!(registry.providers_for_capability("search.query").is_empty());
    }
}
