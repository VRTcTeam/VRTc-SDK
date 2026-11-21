# M7 — Native HTTP Fetcher Engine

**Status:** Initial native implementation integrated into the Rust runtime  
**Engine ID:** `engine.vrtc.fetcher`  
**Capability:** `http.fetch`

## Purpose

The VRTc HTTP Fetcher retrieves one HTTP(S) resource and returns a bounded, provenance-preserving response. It is a transport engine, not a crawler, browser automation system, or content extractor.

## Responsibilities

- Accept absolute HTTP and HTTPS URLs only.
- Reject embedded URL credentials and non-standard ports.
- Resolve hostnames and reject non-public address ranges.
- Pin the resolved address for the individual request to reduce DNS-rebinding risk.
- Disable environment proxy use and automatic redirects.
- Revalidate every redirect target and block HTTPS-to-HTTP downgrade.
- Bound request duration, redirect count, and response body size.
- Preserve requested/final URLs, status, selected response headers, redirect chain, body bytes, and fetch timestamp.
- Return structured errors without echoing credentials or raw transport internals.

## Explicit non-responsibilities

- URL discovery, queueing, crawl depth, and politeness scheduling belong to the URL Frontier.
- JavaScript execution belongs to the Browser & Rendering Engine.
- Robots policy and operator crawl scope must be enforced by the crawler/policy layer before dispatch.
- Main-content parsing, metadata extraction, and document normalization belong to Parsing & Extraction.
- Retry scheduling belongs to the Frontier/Orchestrator, not an unbounded retry loop inside the fetcher.

## Security boundaries

- Only public IP destinations are allowed by default.
- Standard HTTP/HTTPS ports only.
- All DNS answers must pass the public-address policy; the selected answer is pinned into the HTTP client.
- Redirects are manual and each destination is revalidated.
- Response bodies are streamed and stopped when the configured byte ceiling is exceeded.
- The fetcher does not bypass authentication, paywalls, CAPTCHAs, robots rules, rate limits, or other access controls.
- The implementation is an initial single-request fetch engine; production hardening still needs integration with the crawler policy gate, robots cache, host concurrency budgets, telemetry, and persistent fetch records.

## Configuration

- `timeout`: request timeout.
- `max_body_bytes`: maximum response body bytes.
- `max_redirects`: maximum redirect hops (capped at ten).
- `user_agent`: explicit identifying user agent.

## Runtime integration

The engine is statically registered by `VrtcRuntime` and exposed through `fetch_engine()`. It declares the `http.fetch` capability. The public HTTP API route and language SDK methods are future API milestones and are not claimed as implemented here.
