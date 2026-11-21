# M14 — Native Delta & Content Versioning

**Status:** Initial native engine integrated into the Rust runtime and crawler  
**Engine ID:** `engine.vrtc.delta`  
**Capability:** `content.delta`

## Research basis and adoption

M14 combines common change-detection patterns from web archives, incremental crawlers, document stores, and content synchronization systems:

- Stable content fingerprints distinguish unchanged observations from modifications.
- Version history preserves prior content snapshots and observation timestamps.
- Line-oriented diffing provides a bounded, inspectable change summary.
- The crawler indexes a successful extracted document before recording its delta, so a failed index write cannot cause a later retry to be misclassified as already indexed.
- Change detection is a separate subsystem from crawling and indexing; callers can use it for non-web document sources as well.

### Adopted primitives

- **SHA-256** from RustCrypto `sha2 0.11.1` (MIT OR Apache-2.0) for stable content fingerprints.
- **similar 2.7.0** (Apache-2.0) for bounded line-oriented diffing.
- VRTc owns URL identity, normalized fingerprint input, version semantics, history bounds, diff limits, and runtime integration.

## Fingerprint semantics

The fingerprint covers title, description, and extracted content. Each field is whitespace-collapsed and length-prefixed before hashing, avoiding ambiguous field concatenation. Case is preserved.

Consequences:
- Whitespace-only changes do not create a new version.
- Title or description changes are recorded as metadata changes even if body text is unchanged.
- A content change creates a new version and a bounded line diff.
- An unchanged observation updates `last_seen_at` without consuming another version slot.

## Change lifecycle

| Observation | Result | Version behavior |
|---|---|---|
| First snapshot | `Created` | Starts at version 1 |
| Same normalized fingerprint | `Unchanged` | Keeps current version; updates last-seen time |
| Different fingerprint | `Modified` | Increments version and stores a new snapshot |

Each delta record contains canonical URL, version, change kind, previous/current hashes, added/removed line counts, metadata-change flag, diff-truncated flag, and observation timestamp.

## Resource bounds

- Maximum tracked URL count.
- Maximum retained versions per URL.
- Maximum total snapshot bytes.
- Maximum line count considered for detailed diff.
- Older versions are trimmed per URL when the configured history bound is exceeded.
- Capacity exhaustion is explicit; the engine does not silently evict an unrelated URL.

If a diff exceeds the configured line bound, the new snapshot is still recorded and the record marks `diff_truncated`; line counts are set to zero rather than pretending a partial diff is complete.

## Crawler integration

After a successful index upsert, the Crawler records the same canonical URL, title, description, and extracted body with the Delta Engine. The crawl report distinguishes created, modified, and unchanged documents. Delta-store errors are reported separately and do not roll back a successful index write.

## Runtime integration

The runtime statically registers `engine.vrtc.delta` / `content.delta`, exposes `delta_engine()`, and shares the same Delta Engine instance with the Crawler. This avoids separate, inconsistent version stores.

## Boundaries

- Initial history is bounded and in-memory; process restart loses versions.
- No distributed version store, retention scheduler, archive object storage, or restore endpoint yet.
- The diff is a line-oriented summary, not a semantic document diff.
- SHA-256 is used for content identity, not authentication or tamper-proof provenance.
- Crawler still indexes every successful extraction; delta detection currently records change state but does not yet suppress redundant index writes.
