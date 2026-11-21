# M3 — Search Engine Specification

**Status:** Initial Rust implementation baseline  
**Runtime:** Rust  
**Execution model:** Async, bounded concurrency  
**Provider model:** Optional local and remote adapters

## Purpose

The VRTc Search Engine is the VRTc-owned coordinator for query validation, provider selection, concurrent execution, result normalization, filtering, deduplication, ranking, and provenance. It is not a claim that VRTc already operates a proprietary public-web index.

## Responsibilities

1. Normalize whitespace and validate search requests.
2. Enforce configured result limits.
3. Dispatch a request to all registered provider adapters.
4. Bound concurrency and apply a timeout independently to each provider.
5. Isolate provider errors so one failing adapter does not discard successful responses.
6. Normalize provider provenance and discard non-finite scores.
7. Apply language, domain, and publication-date filters.
8. Deduplicate equivalent URLs, rank by VRTc-owned Reciprocal Rank Fusion (RRF), and cap the final result count.
9. Return queried-provider identifiers and structured provider failures with the results.
10. Delegate multi-source aggregation to the native Metasearch capability, which uses Reciprocal Rank Fusion rather than comparing raw scores from unrelated providers.

## Core request and response

SearchRequest contains a query, result limit, optional language, optional domain allowlist, and optional publication-date bounds.

SearchResult contains title, URL, optional snippet and score, provider provenance, optional publication date and language, and non-secret metadata.

SearchResponse contains the normalized query, results, queried providers, and provider-level errors.

Publication date strings must use lexicographically sortable RFC 3339 timestamps. Domain filters match exact hostnames and subdomains. URL deduplication normalizes scheme and hostname case, removes fragments, and trims trailing slashes.

## Provider contract

A provider implements the async SearchProvider interface:

- stable provider identifier;
- one async search operation accepting the canonical request;
- result values returned in the shared VRTc result model.

The engine overwrites provider provenance with the registered adapter ID. Provider descriptors and credentials are managed separately from search requests. Credentials must never be embedded in result metadata or error messages.

## Failure behavior

- Invalid requests fail before dispatch.
- No registered providers returns a structured NOT_REGISTERED error.
- Provider timeout and provider-returned errors are isolated and included in provider_errors.
- An unexpected task failure is represented as a safe provider-task error.
- A provider failure does not cancel other providers.
- Concurrency and timeout limits are configured by the runtime.

## Security and policy

Provider adapters must respect operator authorization, applicable service terms, rate limits, and access policies. The Search Engine does not provide proxy-based access-control bypass. Dynamic provider registration stores a trusted adapter handle and does not load arbitrary native libraries.

## Current boundaries

This milestone does not provide a built-in global web index or browser crawler. Native Metasearch is specified in [M12](m12-native-metasearch.md).
