# VRTc SDK

**VRTc** is a provider-agnostic, polyglot **Multi-Engine Research & Retrieval SDK**.

It coordinates search, discovery, crawling, fetching, browser rendering, extraction, indexing, retrieval, evidence verification, and research workflows through one consistent SDK interface.

> Project status: Rust core contracts, hybrid registry, integrated runtime façade, provider-agnostic Search Engine and native RRF Metasearch, native URL Frontier & Scheduler, policy-constrained HTTP Fetcher, native HTML Extraction, embedded BM25 Document Index, native hybrid Retrieval & Reranking, and native Human Feedback & Evaluation, and native policy-aware Crawler orchestration, and native Delta & Content Versioning, and native Evidence Provenance & Verification, and native Storage & Database Engine, and native Job Queue & Processing Lifecycle, and initial native Research Planner & Orchestration Graph, and initial native Monitoring & Observability telemetry, and initial native Security & Retrieval Policy engine, and initial native WARC Web Archive engine are implemented. Initial Search Engine HTTP router and first TypeScript/Python clients are implemented.

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

The M1 baseline is documented in [Core Contracts & Registry Specification](docs/specifications/m1-core-contracts-and-registry.md). The runtime integration is described in [Runtime & Registry Integration](docs/specifications/m2-runtime-integration.md). The broader native-engine research, including crawler/search systems and human-feedback workflows, is in [Native Engine Landscape Research](docs/research/native-engine-landscape.md).

## Initial subsystem: Search Engine

The first subsystem is a VRTc-owned orchestration layer with provider-neutral contracts.

Implemented foundation:

- async provider interface and runtime registration;
- optional Geekflare Search API adapter for web, news, and image result sources;
- bounded concurrent provider dispatch;
- per-provider timeout and failure isolation;
- query normalization and request validation;
- result normalization, URL deduplication, ranking, and result limits;
- language, domain, and publication-date filters;
- provider provenance and structured provider-level errors.

The VRTc Search Engine coordinates search. A **provider adapter** translates a particular external or local search capability into VRTc's contract. **Metasearch** is the multi-source aggregation capability and remains conceptually distinct from an individual provider.

External search services are optional adapters. The core SDK does not require a paid provider to exist. The current implementation does not include a built-in public-web index or claim to search the internet without a configured provider.

## Native Metasearch & Federated Search

VRTc's native Metasearch capability fans out queries to registered local or remote search sources, isolates per-source timeouts/failures, filters results, deduplicates canonical URLs, and fuses source-local rankings using Reciprocal Rank Fusion. It does not compare raw relevance scores from unrelated providers. Source ranks and original source scores remain available as metadata. Search and Metasearch share one source registry and concurrency policy. See [M12 Native Metasearch Specification](docs/specifications/m12-native-metasearch.md).

## Native URL Frontier & Scheduler

The runtime now statically registers a VRTc-owned URL Frontier Engine. It provides URL canonicalization and deduplication, bounded pending/in-flight queues, priority scheduling, maximum crawl depth, per-host politeness delays, lease expiry and recovery, retry budgets, and terminal completion/failure tracking. The frontier does not fetch pages; HTTP fetching, browser rendering, and parsing remain separate engine responsibilities. See [Native Engine Landscape Research](docs/research/native-engine-landscape.md).

## Native HTTP Fetcher

The runtime also statically registers a VRTc-owned HTTP Fetcher. It supports public HTTP(S) destinations, DNS address checks and pinning, manual redirect revalidation, HTTPS downgrade blocking, bounded time/body size, and response provenance. It is deliberately separate from URL scheduling, browser rendering, and extraction. See [M7 Native HTTP Fetcher Specification](docs/specifications/m7-native-http-fetcher.md).

## Native WARC Web Archive Engine

VRTc now has a bounded in-memory WARC/1.1 response archive with UUID record IDs, capture timestamps, normalized target URLs, filtered response headers, bounded retention, lookup, and WARC export. It archives responses supplied by callers; automatic Fetcher/Crawler capture and durable storage are future work. See [M21 Native Web Archive Specification](docs/specifications/m21-native-web-archive.md).

## Native Security & Retrieval Policy

VRTc now has a shared policy decision engine for HTTP(S) schemes, credential-free URLs, host allow/deny rules, private-address blocking, same-origin crawl scope, depth limits, and explicit robots approval. DNS resolution, redirect revalidation, and robots parsing remain enforced by the Fetcher and Crawler; this policy layer complements rather than replaces those controls. See [M20 Native Security Policy Specification](docs/specifications/m20-native-security-policy.md).

## Native Monitoring & Observability

VRTc now includes bounded in-process counters, gauges, cumulative histograms, and a rolling operational-event buffer. Metric cardinality and labels are bounded; event records store component IDs, event codes, severity, and timestamps rather than raw request payloads. This is an initial telemetry store, not yet an OpenTelemetry exporter, distributed tracing backend, or durable metrics database. See [M19 Native Observability Specification](docs/specifications/m19-native-observability.md).

## Native Research Planner & Orchestration Graph

VRTc now creates bounded deterministic research DAGs from a query, source limit, crawl opt-in, and optional human-review gate. Plans declare capability IDs, dependencies, deterministic topological order, and parallel groups. The planner does not execute jobs or perform network/model calls; M17 queue and the underlying engines remain the execution layer. See [M18 Native Research Planner Specification](docs/specifications/m18-native-research-planner.md).

## Native Job Queue & Processing Lifecycle

VRTc includes a bounded job queue with priority scheduling, worker leases, generation-bound acknowledgements, retries, dead-letter handling, expired-lease recovery, and optional persistence through the shared Storage Engine. Delivery is at-least-once. The current queue coordinates workers within one runtime; it is not a distributed broker or worker executor. See [M17 Native Job Queue Specification](docs/specifications/m17-native-job-queue.md).

## Native Storage & Database Engine

VRTc provides a namespaced key-value storage contract with bounded keys/values/scans, atomic batches, an in-memory default, and an optional local redb backend selected through `RuntimeConfig.storage_path`. The redb backend is experimental/beta upstream. Existing Delta, Evidence, Feedback, and Frontier state stores have not yet been migrated, so this does not make all runtime state durable. See [M16 Native Storage Specification](docs/specifications/m16-native-storage-engine.md).

## Native Evidence Provenance & Claim Verification

VRTc keeps claims, source evidence, explicit assessments, and verification summaries as separate records. It validates source provenance fields, stores excerpts and caller-supplied SHA-256 fingerprints, records human/rule/model-assisted assessments, and reports `Supported`, `Contradicted`, `Mixed`, `NeedsReview`, or `InsufficientEvidence` under transparent thresholds. It does not pretend that source count or an LLM confidence score proves truth. Initial storage is bounded and in-memory. See [M15 Native Evidence Specification](docs/specifications/m15-native-evidence-verification.md).

## Native Delta & Content Versioning

VRTc fingerprints extracted title, description, and content with SHA-256, detects created/modified/unchanged observations, stores bounded version history, and computes bounded line diffs. Whitespace-only changes do not create new versions; metadata changes are tracked separately. The Crawler records snapshots after successful index writes and reports delta outcomes. The initial history store is in-memory. See [M14 Native Delta Specification](docs/specifications/m14-native-delta-versioning.md).

## Native Policy-Aware Crawler

VRTc now composes the Frontier, public-address HTTP Fetcher, robots.txt policy, HTML Extraction, and Tantivy-backed Index into a bounded sequential crawler. It honors user-agent-specific robots rules, caches policies, fails closed on unreachable or malformed robots responses, limits link discovery to the same origin by default, and reports retries and crawl outcomes. It processes HTML/XHTML only; JavaScript rendering and document-format parsing remain separate engines. See [M13 Native Crawler Orchestration](docs/specifications/m13-native-crawler-orchestration.md).

## Native Human Feedback & Evaluation

The runtime statically registers a VRTc-owned human review engine. It creates and lists bounded review tasks, records versioned 0–3 graded relevance judgments, preserves independent reviewer labels, flags material disagreement for adjudication, and evaluates rankings with Precision@k, Recall@k, MRR, nDCG@k, exact agreement rate, and disagreement counts. The initial store is in-memory; authentication, durable persistence, rubric text, reviewer UI, and HTTP/SDK methods remain host/integration responsibilities. See [M11 Native Human Feedback Specification](docs/specifications/m11-native-human-feedback.md).

## Initial HTTP API

The `vrtc-api` crate exposes an embeddable Axum router with `POST /v1/search`, `GET /v1/providers`, and `GET /v1/health`. `router_with_runtime` accepts the unified `VrtcRuntime`, so the HTTP surface uses the same registry-backed Search Engine instance as embedded callers. The host application remains responsible for listener lifecycle, TLS, authentication, authorization, and rate limiting. See [M4 HTTP API Contract](docs/specifications/m4-http-api.md).

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
| M2 | Rust core foundation, registry, and runtime integration | Initial implementation committed |
| M3 | First Search Engine implementation | Initial provider-agnostic engine committed and runtime-integrated |
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
