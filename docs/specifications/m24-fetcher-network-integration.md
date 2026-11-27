# M24 — Fetcher & Network Policy Integration

**Status:** Initial integration of the native Network Engine into the HTTP Fetcher  
**Engines:** `engine.vrtc.fetcher`, `engine.vrtc.network`

## Scope

- The runtime constructs one configured Network Engine and shares it with the Fetcher.
- Every Fetcher request hop asks the Network Engine for an explicit route decision.
- Direct routes use the Network Engine's timeout-bounded DNS resolver and public-address policy.
- The Fetcher pins the selected validated address into the HTTP client for that hop.
- Redirects repeat route planning and DNS validation; HTTPS-to-HTTP downgrade remains blocked.
- Standalone Fetcher construction remains source-compatible and creates a default Network Engine.

## Proxy safety boundary

A matching proxy route is rejected with `PROXY_TRANSPORT_NOT_AVAILABLE` until a transport implementation can enforce destination DNS/IP policy through the proxy connection. The Fetcher does not silently fall back to direct networking and does not delegate unresolved destination names to a proxy.

## TLS boundary

The HTTP client uses reqwest's rustls TLS backend with normal certificate and hostname verification. At the M24 baseline, the Network Engine validated TLS policy before planning HTTPS routes, but the configured minimum version was not yet applied to the connector. M33 closes that specific gap by applying the route's TLS minimum to each reqwest client; see [M33 Fetcher TLS Minimum Enforcement](m33-fetcher-tls-minimum-enforcement.md).

## Non-goals

This milestone does not add browser automation, evade robots/access controls, bypass rate limits, or load third-party native code. Browser backends and provider adapters remain separate contracts.
