# M4 — Initial HTTP API Contract

**Status:** Router baseline implemented  
**Transport:** HTTP + JSON  
**Scope:** Search Engine, provider discovery, and health

## Endpoints

| Method | Path | Purpose |
|---|---|---|
| POST | /v1/search | Execute a normalized search request |
| GET | /v1/providers | List configured search provider identifiers |
| GET | /v1/health | Return process-level API health |

## Search request

The JSON request uses the shared SearchRequest contract:

- query: required string;
- limit: optional integer; zero or omitted uses the configured default;
- language: optional language filter;
- domains: optional hostname allowlist;
- published_after and published_before: optional RFC 3339 timestamp bounds.

## Search response

A successful orchestration returns SearchResponse with normalized query, result list, providers queried, and provider-level errors. A provider failure is not automatically an HTTP failure if other providers complete.

Invalid input maps to HTTP 400. Missing provider configuration maps to 503. Resource exhaustion maps to 429. Provider timeout at the orchestration boundary maps to 504. Other structured errors map to an appropriate 4xx or 5xx status.

## Integration

The crate exposes a router constructor and does not bind a network listener. The embedding application owns listener lifecycle, TLS termination, authentication, authorization, rate limiting, and deployment configuration.

The current API is an initial integration surface, not a complete public hosted service. Authentication and authorization must be applied by the host before exposing it to untrusted clients.
