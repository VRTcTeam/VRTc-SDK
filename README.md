# VRTc SDK

**VRTc** is a provider-agnostic, polyglot **Multi-Engine Research & Retrieval SDK**.

It coordinates search, discovery, crawling, fetching, browser rendering, extraction, indexing, retrieval, evidence verification, and research workflows through one consistent SDK interface.

> Project status: Rust core contracts, hybrid registry, and the first provider-agnostic Search Engine foundation are implemented. Initial Search Engine HTTP router and first TypeScript/Python clients are implemented.

## Project principles

- **VRTc-owned core:** external services are optional integrations, not the identity or mandatory foundation of VRTc.
- **Provider-agnostic:** engines and providers implement stable VRTc contracts.
- **Polyglot SDK:** language clients share canonical API and data contracts.
- **Composable execution:** subsystems can be used independently or orchestrated as a research workflow.
- **Free-first:** local and self-hostable components should be viable defaults; external services may have their own costs and terms.
- **Responsible retrieval:** integrations must respect authorization, applicable access policies, rate limits, and service terms.

## Architecture decisions

| Area | Decision |
|---|---|
| Architecture | Polyglot |
| Core runtime | Rust |
| First subsystem | Search Engine |
| Primary deliverable | SDK |
| Engine/provider registration | Hybrid: static and dynamic |

### Runtime architecture

1. **Language SDKs** — idiomatic clients for supported languages.
2. **API and protocol layer** — stable public interfaces and internal contracts.
3. **VRTc Core Runtime (Rust)** — lifecycle, configuration, registry, and execution foundations.
4. **Research orchestration** — planning, scheduling, jobs, and events.
5. **Engine subsystems** — search, metasearch, crawling, fetching, rendering, extraction, indexing, retrieval, verification, and related capabilities.
6. **Adapters and infrastructure** — optional providers, storage, queues, and other integrations.

The system should begin as a modular runtime. Components should become separate processes or services only when there is a clear operational reason.

## Engine and provider registration

VRTc uses a **hybrid registration model**:

- **Static registration:** built-in engines and providers are registered during runtime initialization.
- **Dynamic registration:** trusted custom engines and provider adapters can be registered while the runtime is running.
- **Unified catalog:** the planner and orchestrator discover capabilities through one registry interface.

Registration does not imply unrestricted native code loading. Dynamic components must pass validation and policy checks. Any future plugin execution model must enforce explicit permissions and isolation; WebAssembly may be evaluated as one plugin boundary.

The M1 baseline is documented in [Core Contracts & Registry Specification](docs/specifications/m1-core-contracts-and-registry.md).

## Initial subsystem: Search Engine

The first subsystem is a VRTc-owned orchestration layer with provider-neutral contracts.

Implemented foundation:

- async provider interface and runtime registration;
- bounded concurrent provider dispatch;
- per-provider timeout and failure isolation;
- query normalization and request validation;
- result normalization, URL deduplication, ranking, and result limits;
- language, domain, and publication-date filters;
- provider provenance and structured provider-level errors.

The VRTc Search Engine coordinates search. A **provider adapter** translates a particular external or local search capability into VRTc's contract. **Metasearch** is the multi-source aggregation capability and remains conceptually distinct from an individual provider.

External search services are optional adapters. The core SDK does not require a paid provider to exist. The current implementation does not include a built-in public-web index or claim to search the internet without a configured provider.

## Initial HTTP API

The `vrtc-api` crate exposes an embeddable Axum router with `POST /v1/search`, `GET /v1/providers`, and `GET /v1/health`. The host application remains responsible for listener lifecycle, TLS, authentication, authorization, and rate limiting. See [M4 HTTP API Contract](docs/specifications/m4-http-api.md).

## Core contracts

The shared Rust contracts include component descriptors, capabilities, structured errors, and search request/result models. The language-neutral contract specification covers:

- ResearchRequest
- ResearchJob
- EngineDescriptor
- ProviderDescriptor
- SearchQuery
- SearchResult
- Source
- Document
- Evidence
- RetrievalResult
- ResearchResult
- EngineEvent
- VRTcError

Contracts should carry stable identifiers, schema versions, timestamps where relevant, structured status and error information, and source provenance.

## Language SDK clients

Initial client packages are available for [TypeScript](sdks/typescript/README.md) and [Python](sdks/python/README.md). Both target the same HTTP contract and expose search, provider discovery, and health operations. The remaining selected language ecosystem will be added incrementally, with each client following the canonical contracts.

## Protocol direction

- **REST + OpenAPI** for the public HTTP API and generated client support.
- **Protobuf + gRPC** for typed internal service communication where needed.
- **WASM Component Model + WIT** as a possible interface for isolated plugins.

These are architecture choices under specification, not claims that the protocols are already implemented.

## Language ecosystem

The selected SDK ecosystem contains 19 entries:

Python, Elixir, Assembly, JavaScript, Java, Markdown, TypeScript, Rust, C#, Go, C++, Kotlin, SQL, Ruby, Lua, PHP, Dart, Julia, and Haxe.

Not every entry is intended to implement every subsystem. Markdown and SQL are supporting interface/data languages rather than general-purpose SDK implementation targets.

## Planned subsystem landscape

1. Crawler Engine
2. Search Engine
3. Metasearch Engine
4. Delta & Change Detection Engine
5. URL Frontier & Scheduler
6. HTTP Fetcher Engine
7. Browser & Rendering Engine
8. Proxy & Network Routing Engine
9. DNS & TLS Infrastructure
10. Parsing & Extraction Engine
11. Indexing Engine
12. Retrieval & Reranking Engine
13. Web Archive Engine
14. Storage & Database Engine
15. Queue & Distributed Processing Engine
16. Research & Agent Orchestration
17. Evidence & Verification Engine
18. Monitoring & Observability Engine
19. Security & Policy Engine
20. Provider Integration Engine

This is a target architecture, not a statement that all subsystems are available today.

## Development roadmap

| Milestone | Focus | Status |
|---|---|---|
| M0 | Architecture and project specification | Baseline established |
| M1 | Core contracts and registry specification | Specification baseline drafted |
| M2 | Rust core foundation and registry implementation | Initial implementation committed |
| M3 | First Search Engine implementation | Initial provider-agnostic engine committed |
| M4 | Initial public HTTP API router | Router baseline implemented |
| M5 | Language SDK clients and provider adapters | Initial TypeScript and Python clients implemented |
| M6 | Additional research and retrieval subsystems | Planned |
| M7 | Hardening, documentation, compatibility, and releases | Planned |

## CI/CD

GitHub Actions runs repository checks and Rust formatting, Clippy, and workspace tests. The repository also includes CodeQL workflow analysis, dependency review, OpenSSF Scorecard reporting, Dependabot updates, and tag-triggered source releases.

A green CI run is required before describing a commit as verified. The current implementation is an early SDK foundation, not a production-ready hosted search service.

## Security

Do not commit credentials or local environment files. Use repository or environment secrets for CI credentials. See [SECURITY.md](SECURITY.md) for vulnerability reporting guidance.

## Repository status

VRTc is under active development. APIs and implementation details may evolve through documented design decisions and versioned contracts.

No provider credentials are required to build the core contracts or Search Engine. Provider integrations must be configured explicitly by the operator.
