# M13 — Native Policy-Aware Crawler Orchestration

**Status:** Initial native engine integrated into the Rust runtime  
**Engine ID:** `engine.vrtc.crawler`  
**Capability:** `crawl.execute`

## Research basis and adoption

M13 combines distinct responsibilities observed across Scrapy, Crawlee, Apache Nutch/StormCrawler, and browser-aware crawler systems:

- A crawler is an orchestrator over a frontier, fetcher, parser/extractor, and indexer; these should remain independent components.
- Frontier state owns URL identity, deduplication, priority, depth, host politeness, leases, retries, and recovery.
- The fetcher owns HTTP request security, redirect revalidation, DNS checks, response limits, and transport provenance.
- Extraction owns content selection, metadata, and outgoing-link normalization.
- Indexing owns the normalized document contract and retrieval-ready storage.
- Human review remains a separate explicit evaluation path.

### Adopted robots.txt parser

VRTc adopts `robotstxt 0.3.0`, an Apache-2.0 Rust port of Google's robots.txt parser/matcher. It has no third-party crate dependencies and exposes a user-agent-aware URL matcher. The dependency is pinned because its upstream release cadence is old; VRTc's wrapper owns fetching, cache policy, conservative failure behavior, and the crawler contract. A broader RFC 9309 conformance suite remains a future validation task.

The crawler follows the Robots Exclusion Protocol in RFC 9309. Robots rules are requests from site operators, not access authorization; the fetcher independently rejects unsafe destinations and redirects.

## Native execution pipeline

```text
Seed URLs
   |
   v
URL Frontier + Scheduler
   |
   v
Robots Policy Gate -----> robots.txt cache
   |
   v
Public HTTP Fetcher
   |
   +---- status/content-type checks
   |
   v
HTML Extraction
   |
   +---- normalized text + metadata + links
   |
   +---- same-origin scope filter (default)
   |
   v
Document Index (Tantivy)
   |
   v
Frontier completion / run report
```

## Responsibilities

- Accept a bounded seed list and enqueue it through the shared URL Frontier.
- Process leases sequentially in bounded crawl runs.
- Check robots.txt per origin before fetching a page.
- Cache successful robots rules for up to 24 hours (configurable, with a hard maximum).
- Enforce a minimum 512 KiB robots response/parser limit.
- Follow robots.txt redirects through the existing Fetcher, which revalidates every redirect destination.
- Apply user-agent-specific rules using the configured product token.
- Fail closed on unreachable robots endpoints, server errors, malformed/oversized robots bodies, and access-denied/rate-limited robots responses.
- Treat robots 404/410 as unavailable and allow access, following RFC 9309's unavailable-resource semantics.
- Fetch only through the public-address-constrained HTTP Fetcher.
- Process only successful HTML/XHTML responses in this milestone.
- Extract readable text, metadata, canonical URL, and normalized links.
- Index extracted documents with stable source URL identity and provenance.
- Enqueue discovered links with incremented depth and same-origin scope by default.
- Report crawl, policy, extraction, indexing, retry, duplicate, and frontier outcomes.
- Apply bounded page, seed, link, robots body, and idle-wait limits.

## Robots policy semantics

| Robots response | M13 behavior |
|---|---|
| 2xx with valid UTF-8 `text/plain` body within size limit | Parse and enforce user-agent rules |
| 404 or 410 | Treat as unavailable; allow crawl |
| 401, 403, 429, other 4xx | Deny conservatively |
| 5xx, network failure, redirect failure | Deny (fail closed) |
| Oversized, invalid UTF-8, or non-text response | Deny (fail closed) |

- Successful rules are cached for no more than 24 hours.
- Failure decisions use a short configurable cache to avoid repeated requests during one run.
- The default product token is `VRTc-Fetcher`; it must correspond to the operator-configured HTTP User-Agent.
- Per-host politeness is enforced separately by the URL Frontier. Robots `crawl-delay` extensions and sitemap directives are not implemented in M13.
- Robots.txt does not grant access to protected resources and does not replace authorization or access controls.

## Run limits and retry behavior

- The crawler is sequential initially. This is deliberate: it avoids adding an independent concurrency layer that could bypass the frontier's host scheduling.
- The maximum pages per run, seed count, discovered links per page, depth, and idle wait are bounded.
- HTTP 429 and 5xx responses are retried with a delay through the Frontier's attempt budget.
- Network/internal fetch failures are retried; validation and policy denials are terminal.
- Non-success non-retryable statuses and non-HTML resources are skipped and recorded.
- Extraction failures are terminal for the current frontier item.
- Index write failures retry the current lease.
- A run can return with pending frontier items if they are not eligible before the configured idle-wait limit; the report exposes frontier state.

## Runtime integration

The runtime statically registers `engine.vrtc.crawler` and `crawl.execute`, and exposes `crawler_engine()`. The crawler receives shared handles to the already registered Frontier, Fetcher, Extraction, and Index engines; it does not create shadow registries or duplicate their policies.

## Boundaries

- No browser JavaScript rendering; Browser & Rendering remains a separate engine.
- No PDF/Office parsing; document parsing remains separate.
- No distributed worker pool or durable queue.
- No automatic cross-origin crawl by default; same-origin only is the default.
- No proxy rotation or access-control bypass.
- No live internet crawling is required by unit tests; tests cover policy matching and configuration behavior.
- Persistent frontier state and persistent robots cache are future storage integrations.
