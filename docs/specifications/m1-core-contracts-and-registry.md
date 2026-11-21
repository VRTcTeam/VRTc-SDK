# M1 — Core Contracts & Registry Specification

**Status:** Specification baseline  
**Milestone:** M1  
**Runtime target:** Rust  
**Architecture:** Polyglot, provider-agnostic  
**Registration model:** Hybrid (static + dynamic)

This document defines the first normative design baseline for VRTc's shared contracts and registry. It is a specification, not an implementation or a claim that runtime functionality already exists.

## 1. Goals and non-goals

### Goals
- Give every engine and provider a stable, machine-readable identity.
- Expose capabilities in a consistent form so the planner can select compatible components.
- Support built-in components registered during startup and trusted components registered at runtime.
- Keep registration, discovery, lifecycle, health, and execution concerns distinct.
- Preserve provider provenance and structured errors across language SDKs.
- Define language-neutral contracts before Rust and polyglot client implementations.

### Non-goals for M1
- Implementing crawler, browser, search, or retrieval execution.
- Loading arbitrary native libraries into the runtime.
- Defining billing, quotas, or subscription behavior.
- Requiring a hosted provider or paid API.
- Splitting every subsystem into a separate process.

## 2. Contract conventions

All externally exchanged contract objects must:
- use stable identifiers independent of display names;
- include a schema version where persisted or exchanged across a process boundary;
- distinguish required fields from optional metadata;
- use UTC RFC 3339 timestamps when a timestamp is required;
- preserve source and provider provenance;
- avoid embedding credentials, access tokens, or secret configuration values;
- remain forward-compatible where practical.

Identifiers are opaque strings. Recommended namespaces:
- engine: `engine.<domain>.<name>`
- provider: `provider.<domain>.<name>`
- capability: `<domain>.<operation>`

Identifiers are case-sensitive and immutable after publication. Display names may change without changing an identifier.

## 3. EngineDescriptor

An `EngineDescriptor` describes an executable VRTc subsystem or engine implementation.

| Field | Requirement | Meaning |
|---|---|---|
| `id` | Required | Stable engine identifier |
| `name` | Required | Human-readable name |
| `version` | Required | Engine implementation version |
| `contract_version` | Required | Compatible VRTc contract version |
| `kind` | Required | Engine category |
| `capabilities` | Required | Supported capability declarations |
| `lifecycle_state` | Required | Current lifecycle state |
| `registration_source` | Required | Built-in or runtime registration |
| `metadata` | Optional | Non-secret descriptive metadata |

An engine descriptor describes capability; it does not itself grant permission to execute the engine.

## 4. ProviderDescriptor

A `ProviderDescriptor` describes a local or external implementation that supplies one or more capabilities to an engine.

| Field | Requirement | Meaning |
|---|---|---|
| `id` | Required | Stable provider identifier |
| `name` | Required | Human-readable provider name |
| `version` | Required | Adapter/provider integration version |
| `contract_version` | Required | Compatible VRTc contract version |
| `capabilities` | Required | Supported operations and constraints |
| `connection_mode` | Required | Local, remote API, or other declared mode |
| `lifecycle_state` | Required | Current lifecycle state |
| `registration_source` | Required | Built-in or runtime registration |
| `metadata` | Optional | Non-secret descriptive metadata |

Credentials and secret values must be supplied through a separate secret/configuration interface. Descriptors may contain a reference to a credential source, never the credential itself.

## 5. Capability model

A capability declaration must identify:
- a stable capability identifier;
- supported operation(s);
- input and output contract versions;
- relevant constraints (for example language, region, result limit, or content type);
- whether the capability is available, degraded, or unavailable.

Capability discovery is descriptive, not an authorization decision. The planner must also consider policy, configuration, health, resource limits, and caller permissions before dispatch.

## 6. Registry interfaces

The runtime exposes one unified catalog backed by two registration paths:

### Static registry
- Registers built-in engines/providers during controlled startup.
- Rejects duplicate identifiers and incompatible contract versions.
- Applies configured enable/disable state before the runtime becomes ready.
- Does not silently replace a built-in descriptor.

### Dynamic registry
- Accepts explicitly trusted descriptors through a runtime registration API.
- Validates identifiers, versions, capabilities, policy, and ownership.
- Supports register, inspect, enable, disable, and unregister operations.
- Rejects duplicate identifiers unless an explicit versioned replacement policy is defined.
- Does not imply arbitrary native-code loading or automatic code execution.

### Unified catalog
The planner and orchestrator query a unified read interface regardless of registration source. Registry operations must be concurrency-safe and must not expose partially applied registration changes.

## 7. Lifecycle model

The initial lifecycle states are:

`DISCOVERED → VALIDATED → REGISTERED → INITIALIZING → READY`

Operational transitions:
- `READY → DEGRADED` when a component remains usable with reduced capability.
- `READY|DEGRADED → DISABLED` through operator or policy action.
- `READY|DEGRADED|DISABLED → SHUTTING_DOWN → STOPPED` during removal or runtime shutdown.
- `INITIALIZING → FAILED` when initialization cannot complete.

Invalid transitions must return a structured lifecycle error. A disabled or failed component must not be selected for new work. In-flight work follows the cancellation and shutdown policy of its execution contract.

## 8. Health and availability

Health is separate from lifecycle state:
- `HEALTHY`
- `DEGRADED`
- `UNHEALTHY`
- `UNKNOWN`

Health checks must have bounded timeouts and must not expose credentials or sensitive provider responses. A health-check failure must be isolated to that provider/engine and must not crash the registry or unrelated executions.

## 9. Error model

Every error has:
- stable `code`;
- safe, human-readable `message`;
- `category`;
- `retryable` boolean;
- optional `component_id`, `operation_id`, and correlation identifier;
- optional structured, non-secret details.

Initial categories:
- `VALIDATION`
- `CONTRACT_INCOMPATIBLE`
- `DUPLICATE_ID`
- `NOT_FOUND`
- `NOT_REGISTERED`
- `LIFECYCLE_INVALID`
- `DISABLED`
- `POLICY_DENIED`
- `HEALTH_CHECK_FAILED`
- `TIMEOUT`
- `CANCELLED`
- `RESOURCE_EXHAUSTED`
- `INTERNAL`

Secrets, authorization headers, raw credentials, and sensitive request payloads must never be copied into error messages or logs.

## 10. Security and trust boundaries

- Dynamic registration requires an explicit trust decision.
- Registration metadata is untrusted input and must be validated.
- Native plugins are not loaded merely because they are registered.
- Any future executable plugin mechanism requires isolation, explicit capabilities, resource limits, and auditability.
- WASM Component Model/WIT may be evaluated as a constrained plugin boundary; it is not automatically trusted.
- Provider access must respect authorization, service terms, applicable access policies, and rate limits.
- Registry and health endpoints must not disclose secret configuration.

## 11. Observability requirements

Registry events should include:
- registration accepted/rejected;
- component initialized/ready/degraded/disabled/stopped;
- capability catalog changed;
- health status changed;
- lifecycle operation failed.

Events should include timestamp, component identifier, event type, correlation ID where applicable, and safe diagnostic context. They must not include secret values.

## 12. Acceptance criteria for M1

M1 specification is considered ready for implementation when:
1. Engine and provider identity rules are documented.
2. Descriptor and capability fields are defined.
3. Static, dynamic, and unified registry responsibilities are separated.
4. Lifecycle and health states are defined independently.
5. Duplicate registration and replacement behavior is specified.
6. Error categories and secret-redaction rules are documented.
7. Trust boundaries for dynamic registration are explicit.
8. Rust implementation can be written without inventing missing contract semantics.

## 13. Implementation sequence after specification approval

1. Define language-neutral schema representations.
2. Implement Rust contract types and validation.
3. Implement static registry.
4. Implement dynamic registration metadata path.
5. Add unified catalog and capability discovery.
6. Add lifecycle and health interfaces.
7. Add focused unit and contract compatibility tests.
8. Expose stable contracts to the first Search Engine implementation.
