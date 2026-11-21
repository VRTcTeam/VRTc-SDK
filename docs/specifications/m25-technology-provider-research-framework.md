# M25 — Technology & Provider Research Framework

**Status:** Implementation baseline  
**Milestone:** M25  
**Scope:** Provider-neutral technology research, integration decisions, conformance, and measurement

## 1. Purpose

M25 establishes a repeatable way for VRTc to understand and integrate technologies rather than merely collect provider names or API endpoints. A technology may be implemented natively, adapted, adopted as a dependency or standard, recreated through an independent compatible implementation, used as-is behind an adapter, or studied through lawful reverse engineering. Every resulting choice must be measured against a documented baseline.

The seven rules are mandatory decision lenses. They are not mutually exclusive and do not imply that every technology must be rebuilt.

## 2. The Seven Rules

1. **Implementasi** — implement a capability in VRTc when ownership, control, security, portability, or performance justify native development. Define behavior and acceptance criteria before implementation.
2. **Adaptasi** — adapt an existing library, engine, protocol, or architecture behind VRTc contracts. Keep upstream boundaries explicit and record patches and compatibility assumptions.
3. **Adopsi** — adopt a mature standard, format, protocol, or component where reuse is safer and more maintainable than reimplementation. Record version, license, support status, and transitive dependencies.
4. **Recreate** — independently recreate externally observable behavior or a documented interface when interoperability or portability requires it. Do not copy proprietary source, protected implementation details, credentials, or confidential material.
5. **Gunakan** — use an existing local engine, executable, hosted service, or provider through a replaceable adapter. External services remain optional; core functionality must declare its offline/local behavior and degradation mode.
6. **Reverse engineering sistemnya** — study system behavior through public documentation, standards, source available under its license, controlled black-box observations, protocol traces from authorized environments, and reproducible experiments. Do not bypass authentication, paywalls, access controls, rate limits, or technical restrictions; do not extract secrets or private data.
7. **Tolak ukur sistematikanya** — measure each candidate using a repeatable workload, declared environment, representative corpus, explicit metrics, uncertainty, and comparable baselines. A benchmark without its methodology and provenance is not an acceptance result.

For each technology, record one primary rule and any secondary rules, with rationale. “Not applicable” is an allowed decision when documented.

## 3. Technology domains in scope

### 3.1 Acquisition and discovery
- Search engines and metasearch/federated search
- Crawlers, spiders, URL frontier/schedulers, sitemaps, feeds
- Scrapers, structured extraction, page/article extraction
- HTTP clients, fetchers, DNS, TLS, proxy transport, caching
- Robots policy, origin politeness, retry, backoff, concurrency

### 3.2 Browser and rendering
- Full browsers and browser automation
- Headless and headed execution
- Native/local browser engines and remote browser services
- Chromium/Blink, WebKit, and Gecko families
- DOM construction, layout, JavaScript execution, screenshots, PDF rendering
- Browser contexts, profiles, cookies, storage, downloads, permissions, sandboxing

### 3.3 Content understanding and conversion
- HTML/XML/JSON parsing
- DOM and semantic extraction
- Article/readability extraction
- HTML-to-Markdown and PDF/document-to-Markdown
- Metadata, links, canonical URLs, structured data, language detection
- OCR and multimodal extraction where separately authorized
- Normalization, deduplication, chunking, provenance preservation

### 3.4 Provider and integration surfaces
- Search, crawler, scraper, browser, extraction, translation/conversion providers
- Local libraries, command-line tools, remote APIs, self-hosted services
- Adapter SDKs, protocol bridges, capability negotiation, fallback routing

### 3.5 Runtime and operations
- Queues, workers, distributed coordination, storage, cache
- Rate limits, quotas, timeouts, circuit breakers, retries
- Telemetry, tracing, health, lifecycle, policy enforcement
- Native, process-isolated, containerized, and WASM execution boundaries

## 4. Integration decision record

Every evaluated technology must have a record containing:

| Field | Requirement |
|---|---|
| technology_id | Stable ID, e.g. technology.browser.webkit |
| name / domain | Human-readable name and taxonomy domain |
| version_scope | Exact tested version or declared version range |
| source_refs | Official docs, standards, source repository, and research references |
| license | License and redistribution/usage implications; unknown blocks adoption |
| primary_rule | One of the seven rules |
| secondary_rules | Zero or more additional rules |
| capabilities | Operations and input/output contract versions |
| deployment_modes | Local, embedded, process, remote, or hosted |
| security_profile | Network, filesystem, process, data, and credential boundaries |
| limitations | Known gaps and unsupported behavior |
| benchmark_profile | Workload, corpus, environment, metrics, baseline |
| decision | adopt, adapt, implement, recreate, use, defer, or reject |
| rationale | Evidence-backed explanation |
| reviewed_at | UTC RFC 3339 timestamp |

Never put secrets, API keys, cookies, private user data, or raw authorization headers in a record.

## 5. Common adapter contract requirements

All adapters must:
- expose stable identity, adapter version, contract version, capabilities, and connection mode;
- declare local/remote execution and required permissions;
- validate inputs and bound outputs;
- preserve provenance: provider/engine ID, source URL, retrieval time, content type, status, and applicable license/attribution metadata;
- propagate cancellation, deadlines, and structured errors;
- enforce configured concurrency, timeouts, origin policy, and resource limits;
- isolate failures so one provider cannot crash unrelated work;
- redact secrets from logs, traces, errors, and diagnostics;
- distinguish unsupported, unavailable, degraded, denied, timed out, and failed states;
- support deterministic configuration and reproducible version reporting.

A descriptor is descriptive, not authorization. Registry trust and policy checks remain mandatory before execution.

## 6. Domain-specific contract surfaces

### Search
Input: query, locale/region where supported, result limit, safe filters, deadline, correlation ID.  
Output: ranked results with title, URL, snippet, source identity, rank/score semantics, retrieval time, and safe metadata. Do not pretend scores from different providers are directly comparable.

### Fetcher / crawler / spider
Input: canonical URL, allowed methods, origin policy, depth/budget, deadline, user-agent policy, and cancellation.  
Output: fetch status, final URL, redirect chain, headers allowlist, content bytes or bounded stream, timestamps, and provenance. Respect robots policy, origin politeness, SSRF protections, DNS/IP policy, and configured scope. Do not treat a crawler as a browser.

### Scraper / extraction
Input: fetched document or authorized rendered snapshot, extraction profile, content limits.  
Output: typed fields, text blocks, links, structured data, confidence/diagnostics, and source spans where available. Preserve missing-vs-empty distinctions and extraction provenance.

### Browser / rendering
Input: isolated browser context, navigation policy, URL/snapshot, execution mode, resource budget, deadline.  
Output: DOM/snapshot, rendered text, screenshot/PDF where enabled, console/network diagnostics under policy, and engine/version identity. Browser navigation and subresource access must pass the same network and SSRF policy as fetchers.

### HTML/document to Markdown
Input: document bytes or structured DOM, base URL, conversion profile, output budget.  
Output: Markdown plus metadata, source mapping/provenance, omitted-content notes, and conversion diagnostics. Preserve headings, lists, tables, links, image alternatives, code blocks, and document boundaries where possible. Never claim lossless conversion where the source format is richer than Markdown.

## 7. Security and legal boundaries

- Respect robots directives, terms, applicable law, copyright, privacy, and service access restrictions.
- No credential harvesting, session theft, access-control bypass, CAPTCHA circumvention, paywall evasion, or rate-limit evasion.
- SSRF protections must validate destination IPs after DNS resolution and across redirects; block loopback, link-local, private, multicast, and reserved ranges unless an explicit trusted local-development policy allows them.
- Browser and document processing are untrusted-input boundaries. Use sandboxing, process isolation, resource limits, safe file handling, and network egress policy.
- Do not execute arbitrary downloaded code or browser extensions as part of provider integration.
- Provider keys and cookies belong in a secret store/config interface, never descriptors or source control.

## 8. Systematic benchmark methodology

Each benchmark report must record:
- implementation and exact version, configuration, OS/runtime, CPU/memory, network conditions;
- workload definition, dataset/corpus origin, sample size, selection method, and data permissions;
- warm-up policy, repetitions, concurrency, timeout, and failure handling;
- raw observations or machine-readable aggregates and the calculation method;
- baseline and comparison candidates under equivalent conditions;
- known confounders, missing data, and confidence/uncertainty.

Required metric families:
- **Correctness:** extraction precision/recall/F1 where labels exist, URL correctness, content fidelity, Markdown structure preservation.
- **Coverage:** successful pages, supported content types, language coverage, browser feature coverage.
- **Performance:** p50/p95/p99 latency, throughput, time-to-first-byte, render time, conversion time.
- **Reliability:** success rate, timeout rate, retry rate, crash rate, degraded availability.
- **Efficiency:** CPU time, peak RSS, bytes transferred, storage growth, cost where applicable.
- **Safety/policy:** denied unsafe destinations, robots/policy compliance, secret leakage checks, resource-limit enforcement.
- **Interoperability:** contract conformance, portability, version compatibility, fallback behavior.

No single aggregate score may hide a security failure or a correctness regression. Security and policy gates are pass/fail gates, not weighted trade-offs.

## 9. Evaluation lifecycle

1. Discover candidate technology and primary sources.
2. Create a technology record and map it to the taxonomy.
3. Review license, maintenance, threat model, and policy compatibility.
4. Map capabilities to VRTc contracts and identify gaps.
5. Select the seven-rule approach and document why.
6. Build a minimal adapter or isolated proof of compatibility.
7. Run focused conformance and benchmark workloads once the implementation is meaningful.
8. Record results, limitations, and decision.
9. Integrate only after security, policy, and contract gates pass.
10. Re-evaluate on major upstream/runtime changes or material regressions.

## 10. Relationship to existing VRTc crates

M25 is an architecture and decision framework over existing components; it does not replace them:
- vrtc-search: search orchestration and search provider boundary.
- vrtc-frontier: URL scheduling and crawl frontier.
- vrtc-fetcher / vrtc-network: network acquisition and transport policy.
- vrtc-crawler: crawl orchestration.
- vrtc-extract: document extraction and normalization.
- vrtc-browser: browser rendering boundary.
- vrtc-registry: descriptors, registration, lifecycle, and catalog.
- vrtc-policy: authorization and policy decisions.
- vrtc-observability: telemetry and diagnostics.
- vrtc-research: planning and orchestration.

Existing implementations remain authoritative until a separate milestone changes their contracts. M25 does not introduce mandatory external providers.

## 11. Acceptance criteria

- Seven rules are documented as distinct, usable decision lenses.
- Technology taxonomy covers acquisition, browser engines, providers, conversion, runtime, and evaluation.
- Decision record captures provenance, licensing, security, capabilities, and benchmark evidence.
- Common adapter requirements are defined without forcing a single vendor.
- Search, fetch/crawl, scrape/extract, browser, and document conversion contracts are scoped.
- Benchmark methodology is reproducible and includes correctness, coverage, performance, reliability, efficiency, safety, and interoperability.
- Legal, privacy, robots, SSRF, sandbox, and secret-handling boundaries are explicit.
- No external provider is mandatory and no unsupported implementation status is claimed.

## 12. Follow-on implementation tracks

- Browser engine abstraction and compatibility matrix (Chromium/Blink, WebKit, Gecko).
- Crawler/spider scheduling and distributed execution research.
- Scraper and extraction adapter conformance.
- HTML/PDF/document-to-Markdown conversion adapters.
- Provider lifecycle, health, quotas, circuit breakers, and fallback routing.
- Benchmark corpus, harness, and regression reporting.

These tracks require their own implementation changes and must not be represented as completed by this specification.
