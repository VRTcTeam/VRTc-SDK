# M2 Runtime & Registry Integration

## Purpose

The `vrtc-runtime` crate is the first executable façade over the shared contracts, hybrid registry, and Search Engine. It provides one in-process runtime instance for embedded applications and the HTTP API.

## Runtime ownership

A `VrtcRuntime` owns:

- a shared `Registry` for engine and provider descriptors;
- the configured `SearchEngine` instance;
- a registration mutex that serializes provider registration through the runtime façade.

Initialization statically registers and activates `engine.vrtc.search` with the `search.query` capability.

## Provider registration

A search provider is registered with both:

1. a `ProviderDescriptor` describing identity, version, connection mode, capabilities, and metadata;
2. a typed `Arc<dyn SearchProvider>` adapter implementing executable search behavior.

The descriptor ID must match the adapter ID. Static and dynamic registrations both pass through the same registry validation and lifecycle transitions. A provider becomes discoverable as ready only after registration and activation. Dynamic registration does not load arbitrary native libraries or execute untrusted code.

If adapter registration fails after descriptor activation, the runtime disables the descriptor so it is not returned as an active capability.

## API integration

The HTTP crate retains `router(search_engine)` for callers that already manage a Search Engine. It also exposes `router_with_runtime(runtime)`, which obtains the Search Engine handle from the unified runtime. Both routes therefore use the same provider execution implementation.

## Current boundary

This is an in-process runtime foundation, not a distributed service host. It does not yet implement the research planner, job manager, event bus, plugin sandbox, or external provider adapters. Applications remain responsible for hosting, authentication, authorization, TLS, and rate limiting.

## Validation scope

The runtime crate includes focused tests for:

- built-in Search Engine registration and readiness;
- provider descriptor/adapter identity matching;
- dynamic provider registration, registry discovery, and actual search execution through the runtime.
