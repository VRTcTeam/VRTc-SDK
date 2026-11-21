# VRTc Native Engine Research: Landscape, Human Feedback, and Cross-Breeding

**Research date:** 2026-11-21  
**Repository:** VRTcTeam/VRTc-SDK  
**Purpose:** identify non-provider reference systems and translate their capabilities into VRTc-owned engines.  
**Excluded as primary research targets:** Exa, Firecrawl, Geekflare, Tavily, and Parallel Search. They remain previously established reference points, not the subject of this landscape pass.

## 1. Research rules

VRTc is not to become a collection of API wrappers. A reference project is studied for its architecture, algorithms, workflows, data contracts, operational behavior, and reusable implementation. VRTc then owns the resulting runtime capability.

Every candidate is classified using the project's six adoption methods:

1. **Implement Engine** — build a VRTc-native engine from the required capability and contracts.
2. **Recreate Engine** — independently reproduce documented behavior or algorithms without copying incompatible/proprietary implementation.
3. **Adopt Engine** — embed or depend on a compatible open-source library/component where it is better than reimplementation.
4. **Human-being system** — investigate human evaluation, annotation, feedback, approval, and correction workflows. The phrase is retained as the project's term; this research treats human-in-the-loop (HITL) systems as the closest documented technical category, not as a claim that a product named “Human-being” has been identified.
5. **Cross-breed systems** — combine complementary patterns into one coherent VRTc pipeline, with clear ownership and replaceable components.
6. **Bring into VRTc** — integrate the result through VRTc engine contracts, runtime, registry, API, SDKs, observability, policy, and documentation.

### Selection constraints

- External APIs are optional. They cannot be required for the core runtime to operate.
- Prefer local/self-hostable and offline-capable building blocks where practical.
- Do not copy source code until the exact repository revision, license, notices, dependencies, and transitive licensing are checked.
- A license badge or third-party comparison article is not legal clearance. Re-check the upstream LICENSE/NOTICE at the exact version selected.
- Respect robots.txt, site terms, access controls, rate limits, and operator authorization. No CAPTCHA, login, paywall, or access-control bypass.
- Avoid claiming a general public-web index exists until VRTc actually crawls, stores, indexes, and serves its own corpus.

## 2. Landscape by subsystem

### 2.1 Independent search systems and metasearch

| Candidate | System type | Capabilities worth studying | VRTc use |
|---|---|---|---|
| [YaCy](https://yacy.net/) | Self-hosted and peer-to-peer web search | Own crawler, scheduler, local index, distributed index exchange, privacy-oriented deployment | End-to-end reference for a VRTc-owned web-search stack; study local-first and distributed modes separately |
| [Stract](https://github.com/StractOrg/stract) | Independent web search engine | Own crawler/index, query syntax, site demotion, customizable ranking optics, trusted-link signals | Search quality controls, source diversity, ranking policy |
| [Marginalia Search](https://docs.marginalia.nu/1_overview/05_system_overview/) | Independent search engine | Partitioned index nodes, domain affinity, index-service separation | Domain-aware partitioning and independent search infrastructure |
| [SearXNG](https://docs.searxng.org/) | Metasearch | Federated engines, categories, source-specific request/response handling, privacy-oriented aggregation | Metasearch architecture and source isolation; do not treat it as VRTc's own web index |
| [LibreY](https://github.com/Ahwxorg/LibreY) / [4get](https://github.com/4get/4get) | Metasearch frontends | Lightweight federated source access and result normalization | Compare request fan-out, normalization, and failure handling |

**Important distinction:** YaCy/Stract/Marginalia are references for independent search/index stacks. SearXNG/LibreY/4get are references for federating existing search sources. These are separate engine classes.

### 2.2 Search indexing, retrieval, and ranking

| Candidate | System type | Capabilities worth studying | VRTc use |
|---|---|---|---|
| [Tantivy](https://docs.rs/tantivy/latest/tantivy/) | Rust search library | Schema, tokenization, inverted index, segments, commit/merge, query execution | Strong candidate for an embedded native lexical index in Rust |
| [Apache Lucene](https://lucene.apache.org/) | Search library | Inverted indexes, analyzers, query model, collectors, scoring | Reference for mature lexical search primitives |
| [OpenSearch](https://opensearch.org/docs/latest/) | Distributed search engine | BM25, vector search, hybrid score normalization, reciprocal-rank fusion, pipelines | Study distributed indexing and hybrid query execution |
| [Apache Solr](https://solr.apache.org/guide/) | Distributed search platform | Solr/Lucene indexing, query parsing, facets, distributed search, ranking | Reference for indexing and search service architecture |
| [Vespa](https://docs.vespa.ai/) | Search and serving engine | Retrieval/ranking separation, rank profiles, phased ranking, tensor/vector features, hybrid retrieval | Strong reference for multi-stage retrieval and reranking |
| [Meilisearch](https://www.meilisearch.com/docs) | Search engine | Typo tolerance, full-text, semantic/hybrid search, filtering, facets | Search UX and tunable relevance |
| [Typesense](https://typesense.org/docs/) | Search engine | Typo tolerance, vector search, federated collections, tunable ranking | Query-time hybrid/federated search |
| [Qdrant](https://qdrant.tech/documentation/) / [Milvus](https://milvus.io/docs) / [Weaviate](https://docs.weaviate.io/) | Vector databases | Dense/sparse/multi-vector retrieval, metadata filters, ANN search | Optional vector retrieval backend, not the entire VRTc retrieval subsystem |

**Architecture finding:** retrieval should maximize recall cheaply, then ranking should spend more compute on a smaller candidate set. Hybrid lexical + semantic retrieval is useful because lexical-only can miss semantic matches and vector-only can miss exact terms. Vespa's phased ranking and OpenSearch's score normalization/RRF are useful patterns; weights must be evaluated on VRTc data rather than copied blindly.

### 2.3 Crawlers, URL discovery, and frontier scheduling

| Candidate | Primary strength | Capabilities worth studying | VRTc use |
|---|---|---|---|
| [Scrapy](https://docs.scrapy.org/en/latest/topics/architecture.html) | Modular Python crawler | Engine, scheduler, downloader, middleware, spiders, item pipelines, persistent queues | Core lifecycle and middleware boundaries |
| [Crawlee](https://crawlee.dev/) | Production crawling in JS/TS and Python | Request queues, autoscaling, sessions, retries, HTTP/browser crawlers | Queue lifecycle, session isolation, adaptive concurrency |
| [Apache StormCrawler](https://stormcrawler.apache.org/docs/) | Distributed streaming crawler | URL frontier, stream processing, politeness, parser/indexing modules, external storage | Durable frontier and continuous crawl model |
| [Apache Nutch](https://nutch.apache.org/) | Extensible batch crawler | Crawl database, fetch segments, scoring, deduplication, plugin architecture | Large-crawl lifecycle and plugin boundaries |
| [Heritrix](https://github.com/internetarchive/heritrix3) | Web-archive crawler | Long-running archival crawls, scope and politeness controls | Archive-grade crawl profiles |
| [Colly](https://github.com/gocolly/colly) | Go crawler framework | Concurrent callbacks, extensions, collectors | Lightweight high-throughput crawl patterns |
| [Katana](https://github.com/projectdiscovery/katana) | URL discovery crawler | Link extraction, scope, headless discovery mode | Dedicated discovery engine; only authorized public targets |
| [Spider](https://github.com/spider-rs/spider) | Rust web crawler | Rust-native concurrent crawl and extraction primitives | Evaluate for native Rust adoption |
| [Crawl4AI](https://github.com/unclecode/crawl4ai) | Browser-aware extraction crawler | Browser-backed crawling, markdown-oriented output, extraction workflows | Rendering-to-extraction pipeline |

**Architecture finding:** the frontier must be a first-class subsystem. It owns canonical URL identity, deduplication, priority, depth, host-level politeness, retry/backoff, lease/recovery, crawl scope, and completion state. Fetchers and parsers should not own scheduling policy.

### 2.4 Browser automation and rendering

| Candidate | Capabilities worth studying | VRTc use |
|---|---|---|
| [Playwright](https://playwright.dev/docs/intro) | Browser contexts, isolated sessions, page lifecycle, network events, screenshots, multiple browser engines | Primary browser-control reference and possible adopted runtime |
| [Puppeteer](https://pptr.dev/) | Chrome/Chromium automation, DevTools protocol | Alternative browser backend |
| [Selenium](https://www.selenium.dev/documentation/) | WebDriver and cross-browser automation | Compatibility/reference path |
| [Browserless](https://docs.browserless.io/) | Self-hosted browser workers, WebSocket, REST, session and health management | Operational model for remote browser workers; optional infrastructure |

Browser rendering is a fetch mode, not the crawler itself. VRTc must keep navigation policy, URL scheduling, browser process management, and extraction as separate components.

### 2.5 Extraction, parsing, and document normalization

| Candidate | Capabilities worth studying | VRTc use |
|---|---|---|
| [Trafilatura](https://trafilatura.readthedocs.io/) | Main-text extraction, metadata, links, tables, feeds/sitemaps, multiple output formats | Strong candidate for an adopted extraction backend or benchmark baseline |
| [jusText](https://github.com/miso-belica/ jusText) | Boilerplate removal while retaining sentence-like text | Secondary extraction strategy |
| [Readability](https://github.com/mozilla/readability) | Article extraction and DOM cleanup | Browser-side extraction fallback |
| [Apache Tika](https://tika.apache.org/) | Parsing document formats and metadata | Non-HTML document parsing |
| [Crawl4AI](https://docs.crawl4ai.com/) | Browser-aware extraction, structured output, markdown workflows | Cross-layer reference for render + extract |
| [newspaper4k](https://github.com/AndyTheFactory/newspaper4k) / [Goose3](https://github.com/goose3/goose3) | Article-oriented extraction | Comparative extraction candidates |

Extraction must return structured fields with provenance and extraction confidence, not only a Markdown string. Preserve original response metadata and avoid fabricating missing dates/authors.

### 2.6 Change detection, archives, and temporal web

| Candidate | Capabilities worth studying | VRTc use |
|---|---|---|
| [changedetection.io](https://github.com/dgtlmoon/changedetection.io) | Scheduled checks, content filters, change notifications | Change detection workflows |
| [urlwatch](https://github.com/thp/urlwatch) | Declarative URL monitoring and filters | Lightweight monitor model |
| [pywb](https://pywb.readthedocs.io/) | WARC/ARC indexing and replay, archive collections | Native web archive/replay interface |
| [Webrecorder](https://webrecorder.net/) / [Browsertrix Crawler](https://github.com/webrecorder/browsertrix-crawler) | Browser-based high-fidelity web capture | Dynamic-page archival and WARC/WACZ patterns |
| [warcio](https://github.com/webrecorder/warcio) | Streaming WARC/ARC read/write | Archive serialization primitives |
| [Common Crawl](https://commoncrawl.org/) | Large public crawl datasets and WARC corpus | Optional corpus/data source; not a VRTc runtime dependency |

Delta detection should compare normalized content and metadata, preserve version history, and distinguish meaningful changes from template noise. Archive replay and change detection are related but distinct.

### 2.7 Queue, workflow, and distributed execution

| Candidate | Capabilities worth studying | VRTc use |
|---|---|---|
| [Temporal](https://docs.temporal.io/) | Durable workflow state, retries, timers, recovery | Research job orchestration semantics |
| [Prefect](https://docs.prefect.io/) / [Dagster](https://docs.dagster.io/) | Data/AI workflows, task dependencies, retries, human approval | Workflow patterns and operator review |
| [Celery](https://docs.celeryq.dev/) | Task queues, workers, retries, scheduling | Python worker patterns |
| [NATS](https://docs.nats.io/) | Messaging, request/reply, JetStream persistence | Lightweight event and queue transport |
| [RabbitMQ](https://www.rabbitmq.com/docs) | Durable queues, routing, acknowledgements | Queue semantics |
| [Kafka](https://kafka.apache.org/documentation/) | Durable event streams and replay | High-volume event pipeline |
| [Ray](https://docs.ray.io/) | Distributed Python execution | Parallel research/ML task execution |

VRTc should first define its own job/event contracts and in-process execution semantics. External queue engines can later be adopted behind a queue adapter; they should not leak their message models into public SDK contracts.

### 2.8 Human-being: human-in-the-loop and relevance feedback

No single product named “Human-being” has been confirmed from this wording. The closest established engineering category is **human-in-the-loop (HITL)**.

| Candidate/pattern | Human role | VRTc use |
|---|---|---|
| [Argilla](https://docs.argilla.io/) | Labeling, correction, ranking/preferences, dataset curation and monitoring | Human feedback workbench and relevance-judgment collection |
| [Label Studio](https://labelstud.io/guide/) | Annotation tasks, reviewers, labeling workflows | General annotation and review UI patterns |
| Search quality raters | Assess query-result relevance, satisfaction, source quality, diversity | Human evaluation dataset for ranking and retrieval |
| Editorial/source review | Confirm source credibility, evidence sufficiency, and conflicting claims | Evidence verification and research quality gate |
| Operator approval | Approve crawl scope, sensitive domains, and costly jobs | Policy and safety checkpoint |

**VRTc human-feedback loop:**

1. The engine returns ranked results and provenance.
2. A reviewer sees the original query, result, source, extracted evidence, and why the item was ranked.
3. The reviewer labels relevance, factual support, freshness, duplication, source quality, or policy issue.
4. Labels are stored as versioned judgments with reviewer identity/role, timestamp, and rubric version.
5. Evaluation computes metrics such as Precision@k, Recall@k, nDCG@k, MRR, and inter-rater agreement where applicable.
6. Ranking changes are proposed and evaluated against a frozen judgment set before rollout.
7. Feedback is not silently treated as ground truth: disagreement and uncertainty remain visible.

Human review must not become a hidden dependency for every query. It is an explicit evaluation, exception-handling, and approval path. User feedback and expert judgments must be separated by provenance and confidence.

## 3. Cross-breeding design: VRTc-owned pipeline

The useful combination is not to merge all projects into one monolith. It is to compose their strongest patterns behind VRTc-owned contracts:

```text
Research Request
      |
      v
Research Planner + Policy Gate
      |
      v
Parallel Orchestrator / Durable Job
      |
      +--> Search + Metasearch
      |       |
      |       +--> Native Web Index (Tantivy/Lucene-inspired)
      |       +--> Independent Search Sources (optional)
      |
      +--> URL Discovery
      |       |
      |       v
      |    URL Frontier + Scheduler
      |       |
      |       v
      |    HTTP Fetcher -----> Browser Renderer
      |       |                     |
      |       +----------+----------+
      |                  v
      |           Parsing / Extraction
      |                  |
      |           Delta / Version Store
      |                  |
      |           Indexing / Retrieval
      |                  |
      +------------------+
                         v
                Fusion + Reranking
                         |
                 Evidence / Verification
                         |
                 Human Review (when needed)
                         |
                    Synthesis
                         |
                  Result + Provenance
```

### Cross-breed rules

- **Scrapy + StormCrawler + Crawlee:** combine clear component boundaries, durable frontier semantics, polite scheduling, queue recovery, and browser-aware sessions.
- **YaCy + Stract + Marginalia + Tantivy:** combine independent crawl/index ownership, configurable ranking signals, domain-aware indexing, and a Rust-native lexical index candidate.
- **Vespa + OpenSearch + Qdrant:** combine multi-stage ranking, score normalization/RRF, lexical/vector hybrid retrieval, and metadata filtering.
- **Trafilatura + Tika + Readability:** combine HTML article extraction, document parsing, and fallback extraction strategies.
- **pywb + Webrecorder + change-detection tools:** combine durable capture, temporal versioning, archive replay, and meaningful-change detection.
- **Temporal/Prefect + NATS/RabbitMQ/Kafka:** combine durable job semantics with replaceable event/queue transport.
- **Argilla + Label Studio + search-rater practices:** combine structured human judgments, reviewer workflows, and measurable ranking feedback.

## 4. Adoption method per layer

| VRTc subsystem | Implement | Recreate | Adopt candidates | Human contribution |
|---|---|---|---|---|
| Search | Query contracts, native query planner, ranking policy | Query syntax, filters, result fusion | Tantivy initially; evaluate Lucene/OpenSearch as alternatives | Relevance judgments |
| Metasearch | Fan-out, timeout, dedupe, fusion, provenance | SearXNG-style engine abstraction | No mandatory hosted source | Source-quality review |
| Crawler | VRTc frontier contracts, scope/policy, state machine | Scrapy/StormCrawler scheduling semantics | Evaluate Spider (Rust), Scrapy, Crawlee | Crawl-scope approvals |
| Fetcher | Request policy, canonical response model, retries | Middleware and transport lifecycle | reqwest initially; optional specialized clients | Exception review |
| Browser | Session contracts, lifecycle, isolation | Context/page workflow | Playwright first candidate | Manual reproduction when needed |
| Extraction | Unified document schema, confidence, provenance | Fallback and boilerplate strategies | Trafilatura/Tika/Readability after license review | Extraction correction |
| Indexing | VRTc document schema, lifecycle, version handling | Segment/commit/merge lifecycle | Tantivy candidate | Dataset curation |
| Retrieval/rerank | Candidate generation, fusion, rerank interface | RRF and phased ranking | Qdrant/OpenSearch/Vespa optional | Relevance labels |
| Delta/archive | Version contract, change scoring, retention policy | Normalized diff and replay flow | pywb/warcio | Change significance review |
| Orchestration | Job/event contract, planner, cancellation | Durable workflow semantics | Temporal/Prefect/NATS optional | Human approval tasks |
| Evidence | Provenance graph, claim/source links, conflict detection | Evidence review workflow | No external dependency required | Expert verification |

This table is a research recommendation, not a final commitment to every dependency. Final adoption requires version-specific compatibility, maintenance, license, security, and operational review.

## 5. Native VRTc implementation sequence

1. **Foundation contracts:** URL, CrawlRequest, CrawlJob, CrawlLease, CrawlResult, Document, DocumentVersion, Evidence, RelevanceJudgment, ReviewTask, EngineEvent.
2. **URL Frontier & Scheduler:** canonicalization, deduplication, priority, depth, host politeness, retry/backoff, lease expiry/recovery, cancellation, bounded queues.
3. **HTTP Fetcher:** bounded requests, content limits, timeout, redirects, content-type checks, robots/policy enforcement, response provenance.
4. **Parsing & Extraction:** HTML parser, main-content extraction, metadata extraction, links, PDF/document parsing, confidence and fallback strategies.
5. **Metasearch:** source-neutral fan-out, per-source policy, timeouts, dedupe, rank fusion, source provenance.
6. **Native lexical index:** embedded Rust index candidate (Tantivy) behind VRTc's own IndexEngine contract.
7. **Browser & Rendering:** isolated Playwright worker backend behind a VRTc BrowserEngine contract.
8. **Retrieval & Reranking:** lexical + optional vector retrieval, RRF/score fusion, phased reranking, offline evaluation.
9. **Delta & Web Archive:** versioned snapshots, WARC/WACZ support, meaningful-change detection and replay.
10. **Human feedback:** relevance judgment API, review queue, rubric versioning, offline metrics and controlled ranking experiments.
11. **Orchestration and distributed scale:** durable jobs, event streams, pluggable queue transport, cancellation and recovery.
12. **Evidence and synthesis:** claim-source graph, contradiction checks, evidence completeness, citations and review escalation.

Each stage must be integrated into the existing Rust runtime and registry, then exposed through stable API/SDK contracts. The engine implementation remains VRTc-owned even when it adopts a library internally.

## 6. License and adoption risk notes

- VRTc is currently AGPL-3.0-or-later. Compatibility is not automatically established by the fact that an upstream project is “open source”.
- Apache-2.0, MIT, BSD, GPL, AGPL, and commercial/source-available terms have materially different obligations. Review each dependency's exact LICENSE, NOTICE, source distribution, linking model, and hosted deployment implications.
- The Meilisearch repository distinguishes its MIT-licensed Community Edition from separately governed Enterprise features; do not assume all features share one license.
- SearXNG and Browsertrix have copyleft considerations that require explicit review before embedding or redistributing.
- A dependency may be adopted as an optional process/service rather than copied into the Rust workspace, but that does not eliminate license, security, or operational review.
- No upstream source has been copied by this research document.

## 7. Source notes

Primary project documentation and repositories reviewed:

- SearXNG engine model and metasearch: https://docs.searxng.org/dev/engines/engine_overview.html and https://docs.searxng.org/user/about.html
- YaCy architecture and decentralized search: https://yacy.net/ and https://yacy.net/docs/
- Stract independent index and ranking customization: https://github.com/StractOrg/stract
- Marginalia index architecture: https://docs.marginalia.nu/1_overview/05_system_overview/
- Scrapy architecture and scheduler: https://docs.scrapy.org/en/latest/topics/architecture.html and https://docs.scrapy.org/en/latest/topics/scheduler.html
- StormCrawler frontier and pipeline: https://stormcrawler.apache.org/docs/
- Tantivy indexing and query architecture: https://docs.rs/tantivy/latest/tantivy/
- OpenSearch hybrid retrieval: https://docs.opensearch.org/latest/vector-search/ai-search/hybrid-search/
- Vespa ranking and hybrid search: https://docs.vespa.ai/en/basics/ranking.html and https://docs.vespa.ai/en/learn/tutorials/hybrid-search.html
- Typesense search features: https://typesense.org/docs/overview/features.html
- Trafilatura extraction and evaluation: https://trafilatura.readthedocs.io/en/stable/index.html and https://trafilatura.readthedocs.io/en/latest/evaluation.html
- pywb archive replay: https://pywb.readthedocs.io/en/latest/manual/usage.html
- Argilla human feedback: https://docs.argilla.io/latest/
- Browserless self-hosted browser infrastructure: https://docs.browserless.io/enterprise/open-source

## 8. Research status

This is a multi-angle landscape and architecture synthesis, not an exhaustive audit of every crawler/search product on the internet. Candidate discovery is intentionally broad; implementation selection must be narrower and based on source-level review. The first native engineering target after this research is the **URL Frontier & Scheduler**, because every crawler, fetcher, archive, and change-detection workflow depends on reliable URL identity, scheduling, politeness, and recovery.
