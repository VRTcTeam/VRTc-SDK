# M9 — Native Document Index & Lexical Retrieval

**Status:** Initial native implementation integrated into the Rust runtime  
**Engine ID:** `engine.vrtc.index`  
**Capability:** `index.documents`

## Research basis

- Tantivy is a Rust full-text indexing library inspired by Lucene and implements inverted-index search with BM25 scoring.
- Lucene/OpenSearch/Vespa research established the separation between document indexing, candidate retrieval, and later-stage ranking.
- Tantivy 0.24.0 is MIT-licensed and supports the existing Rust 1.80 workspace baseline; newer 0.26/0.27 releases require a newer toolchain.

Tantivy is adopted as the embedded index primitive. VRTc owns the document model, schema contract, upsert/delete behavior, query boundary, result/provenance model, and runtime integration. Tantivy is not exposed as the public SDK contract.

## Responsibilities

- Maintain a VRTc document schema with stable IDs, URLs, title, description, body, language, publication time, and source.
- Support in-memory indexing or a local persistent index directory.
- Upsert bounded document batches by stable VRTc ID.
- Commit a batch once and reload the reader for deterministic visibility.
- Delete documents by stable ID.
- Run lexical BM25 queries across title, description, and body.
- Return stored source fields and a bounded text excerpt.
- Expose index document counts.
- Enforce batch size, document byte size, query length, result limit, and writer memory constraints.

## Explicit non-responsibilities

- URL crawling, fetching, and HTML extraction remain separate engines.
- Distributed replication and cluster coordination are not implemented in this embedded index crate.
- Vector embeddings and semantic retrieval are separate optional backends and future capabilities.
- Multi-stage reranking and evidence scoring are separate retrieval/ranking engines.
- The current query interface does not claim full OpenSearch, Solr, or Vespa feature parity.

## Persistence and schema compatibility

- Default runtime configuration uses an in-memory index.
- A configured local directory creates or opens a Tantivy index.
- Existing directories must contain the required VRTc schema fields; incompatible schemas fail closed rather than silently rewriting stored data.
- Schema evolution requires an explicit migration/versioning plan before production use.

## Runtime integration

The engine is statically registered by `VrtcRuntime` and exposed through `index_engine()`. It declares the `index.documents` capability with batch upsert, delete, lexical search, and stats operations. Public API and SDK routes remain future milestones.
