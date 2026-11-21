# M10 — Native Hybrid Retrieval & Reranking

**Status:** Initial native implementation integrated into the Rust runtime  
**Engine ID:** `engine.vrtc.retrieval`  
**Capability:** `retrieval.fuse`

## Research basis

- OpenSearch documents Reciprocal Rank Fusion (RRF), which combines result lists by rank rather than mixing incomparable raw scores.
- Vespa separates candidate retrieval from ranking phases and recommends applying more expensive ranking to a smaller candidate set.
- VRTc adopts these patterns as its own fusion contract, using its local Tantivy-backed index for lexical candidates and accepting optional semantic candidates from a future local vector engine.

## Responsibilities

- Retrieve lexical candidates from the VRTc Document Index.
- Accept bounded semantic candidate lists without requiring a hosted semantic API.
- Deduplicate candidates by normalized URL, falling back to stable document ID.
- Fuse lexical and semantic ranks with configurable weighted RRF.
- Apply a small deterministic query-term coverage signal as a lightweight reranking feature.
- Preserve lexical/semantic rank positions, raw lexical score, semantic similarity, contributing channels, fusion score, and final rank.
- Enforce candidate, query, result, and score bounds.

## Formula

For candidate (d):

`score(d) = w_l/(k+r_l(d)) + w_s/(k+r_s(d)) + \lambda \cdot coverage(d,q)`

A missing channel contributes zero. The configured lexical and semantic weights are normalized to sum to one. The default rank constant is 60. The term-coverage contribution is intentionally small and deterministic.

## Important semantics

- RRF uses positions, not raw BM25 and vector similarity values. This avoids directly comparing scores that may live on different scales.
- RRF scores are query-local and should not be compared as absolute relevance probabilities across different queries.
- Semantic similarity is preserved for diagnostics, but does not replace semantic rank in the RRF formula.
- The initial semantic input is a contract for a future local vector engine; this crate does not generate embeddings or call an external API.
- The current term-coverage feature is a lightweight deterministic tie-breaker, not a learned cross-encoder.

## Explicit non-responsibilities

- Embedding generation and vector indexing belong to a separate Vector Engine.
- Human relevance judgments and offline metric evaluation belong to the Human Feedback/Evaluation Engine.
- Evidence verification and source credibility scoring remain separate subsystems.
- Distributed retrieval and cross-shard rank coordination are not implemented in this embedded runtime stage.

## Runtime integration

The engine is statically registered by `VrtcRuntime` and exposed through `retrieval_engine()`. It declares `retrieval.fuse` with retrieve, fuse, and rerank operations. Public API routes and language SDK methods remain future milestones.
