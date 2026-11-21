# M12 — Native Metasearch & Federated Rank Fusion

**Status:** Initial implementation integrated into the VRTc Search Engine runtime  
**Engine ID:** `engine.vrtc.metasearch`  
**Capability:** `metasearch.aggregate`

## Research basis

- **SearXNG** demonstrates metasearch as concurrent fan-out across independently configured engines, with per-engine configuration, categories, locale, time-range support, and typed result areas. VRTc adopts source isolation and capability-aware federation, while keeping its own contracts and runtime.
- **YaCy** demonstrates local and decentralized indexing modes that do not require a single central search provider. VRTc adopts the architectural principle of source independence; peer-to-peer index exchange is not part of this milestone.
- **Stract** documents an independent crawler/index, advanced query syntax, customizable ranking signals, and user-controlled ranking optics. VRTc adopts the idea that ranking signals should be inspectable and configurable, without copying its implementation.
- **Marginalia** documents domain-affine index partitioning and the distinction between an Internet search engine and a general document search index. This informs VRTc's later partitioning and web-index design.

## Native implementation

The existing provider-agnostic Search Engine already owns bounded concurrent source dispatch, per-source timeouts, failure isolation, query normalization, filtering, and provenance. M12 formalizes and registers the Metasearch capability on this runtime and replaces cross-provider raw-score comparison with VRTc-owned Reciprocal Rank Fusion (RRF).

### Execution pipeline

1. Normalize and validate the request.
2. Snapshot registered local/remote source adapters.
3. Fan out the query with bounded concurrency and per-source timeouts.
4. Isolate failed sources without discarding successful responses.
5. Apply request language, domain, and publication-date filters.
6. Preserve each source's own ordering and compute a source-local rank.
7. Canonicalize URLs and deduplicate repeated documents within each source.
8. Fuse each candidate's source ranks using RRF.
9. Preserve source count, source ranks, original provider scores, and primary source provenance.
10. Sort deterministically and apply the requested result limit.

## Reciprocal Rank Fusion

For document (d), source list (i), and source-local rank (r_i(d)):

[
RRF(d) = sum_i rac{1}{k + r_i(d)}
]

- (k) is a positive configurable constant; default is 60.
- Ranks begin at 1.
- Missing source contributions are zero.
- Raw provider scores are not compared across sources because their scales and calibration may differ.
- The result's `score` is the VRTc RRF score, not a probability or a source's raw relevance score.

### Provenance metadata

The result metadata contains:

- `vrtc.metasearch.rrf_score`
- `vrtc.metasearch.source_count`
- `vrtc.metasearch.provider_ranks`
- `vrtc.provider_score.<provider-id>` when a source supplies a finite raw score

The canonical `provider_id` remains the deterministic primary source for compatibility; the full contributing source/rank set is separately preserved.

## Runtime registration

The runtime registers `engine.vrtc.metasearch` and `metasearch.aggregate`. Search and Metasearch share the same source catalog and execution object so provider registration, timeout policy, and concurrency limits cannot drift between duplicate registries. The runtime exposes `metasearch_engine()` as an explicit capability handle.

## Boundaries

- This is federated metasearch over configured adapters; it is not a standalone global web index.
- No external search API is mandatory.
- No attempt is made to scrape around login, paywall, CAPTCHA, access denial, or provider restrictions.
- Source-specific categories, query syntax, paging, and locale capabilities require descriptor-level contracts in a subsequent capability-negotiation milestone.
- Result diversity controls, trusted-domain optics, and calibrated relevance evaluation are future ranking features.
- The Human Feedback Engine can evaluate M12 output using versioned relevance judgments and offline ranking metrics.
