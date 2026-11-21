# VRTc Technology Evaluation Matrix

This matrix is the working inventory for research under the seven-rule framework. It intentionally separates a technology family from a specific implementation or provider. A row is a research target, not an endorsement or a claim of integration.

## Evaluation status vocabulary

- **unreviewed** — candidate identified; no evidence review yet.
- **researching** — primary documentation and behavior are being mapped.
- **evaluated** — evidence and benchmark profile recorded.
- **adopted** — approved for use with a pinned version and license review.
- **adapted** — wrapped or modified behind VRTc contracts.
- **implemented** — VRTc-owned implementation exists.
- **recreated** — independent compatible behavior exists and has conformance evidence.
- **deferred** — valid candidate, but prerequisites or resources are missing.
- **rejected** — explicit reason recorded.

## Technology families

| Family | Research targets | Core questions | Required measures |
|---|---|---|---|
| Search | native search, metasearch, federated search, provider APIs | query semantics, ranking, provenance, limits, failure isolation | relevance, coverage, latency, result diversity |
| Crawling | breadth/depth crawlers, focused crawlers, distributed crawlers | frontier, deduplication, politeness, recrawl, scope | pages/sec, coverage, duplicate rate, policy compliance |
| Spidering | link discovery, sitemap/feed discovery, graph traversal | URL normalization, traps, depth, canonicalization | discovery yield, trap resistance, frontier growth |
| Scraping | DOM selectors, structured extraction, article extraction | schema stability, missing fields, dynamic content | precision/recall, robustness, maintenance cost |
| Fetching | HTTP clients, transport stacks, proxy-aware clients | redirects, TLS, DNS/IP checks, streaming, cancellation | latency, throughput, correctness, safety gates |
| Browser automation | automation protocols and browser control | context isolation, navigation policy, determinism | compatibility, task success, resource use |
| Browser engines | Chromium/Blink, WebKit, Gecko | engine support, APIs, layout differences, sandbox | web-platform coverage, render fidelity, stability |
| Headless execution | headless browser modes, remote browser backends | parity with headed mode, resource limits, isolation | render fidelity, startup time, memory |
| Parsing | HTML5 parsers, XML/JSON parsers, DOM implementations | malformed input, encoding, standards behavior | parse correctness, throughput, memory |
| Markdown conversion | HTML-to-Markdown, PDF/document-to-Markdown | structure preservation, links, tables, code, source maps | structural fidelity, readability, conversion time |
| Document processing | PDF, office formats, EPUB, OCR | extraction order, tables, scanned pages, metadata | text coverage, layout fidelity, error rate |
| Provider integration | local libraries, remote APIs, self-hosted services | contract fit, versioning, auth separation, portability | conformance, availability, fallback success |
| Runtime | workers, queues, caches, process/WASM isolation | backpressure, crash containment, resource budgets | throughput, tail latency, recovery |
| Observability | metrics, logs, traces, health checks | correlation, redaction, cardinality, overhead | diagnostic coverage, overhead, secret leakage |

## Seven-rule decision worksheet

For each candidate, complete:
1. Primary rule (one): Implementasi / Adaptasi / Adopsi / Recreate / Gunakan / Reverse engineering sistemnya / Tolak ukur sistematikanya.
2. Secondary rules (zero or more).
3. Stable technology ID and exact candidate/version.
4. Official documentation, standards, source repository, and research references.
5. License, maintenance state, security advisories, and supported platforms.
6. Capability mapping to VRTc contracts.
7. Data, network, process, filesystem, and credential boundaries.
8. Reproducible workload and metrics.
9. Decision, rationale, limitations, and follow-up owner.

## Baseline comparison rules

- Compare the same input corpus and equivalent configuration.
- Report p50, p95, and p99 for latency; include throughput and failure rate.
- Keep correctness and safety gates separate from performance scores.
- Report resource consumption and the machine/runtime used.
- Pin candidate versions; do not compare moving branches.
- Preserve raw benchmark output and record transformations.
- Mark unavailable metrics as unavailable; never fabricate a score.
- Re-run only when a code change, dependency update, environment change, or regression question justifies it.

## Research and compatibility boundaries

Reverse engineering is limited to lawful, authorized observation and interoperability research. Do not bypass access controls, defeat anti-abuse mechanisms, harvest credentials, or copy proprietary source. Provider-specific restrictions and robots/policy decisions must be represented in the adapter's capability and execution policy.
