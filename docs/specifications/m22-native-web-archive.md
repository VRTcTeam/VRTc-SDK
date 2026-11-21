# M22 — Native WARC Web Archive Engine

**Status:** Initial bounded in-memory WARC/1.1 response archive integrated into the Rust runtime  
**Engine ID:** `engine.vrtc.archive`  
**Capability:** `archive.warc`

## Research basis

M22 adopts the WARC record model used for web-resource preservation. The engine serializes response captures as WARC/1.1 response records containing record identity, capture date, target URI, HTTP response metadata, and body bytes. It is a VRTc-owned archive subsystem and does not call an external archive API.

The implementation is deliberately bounded and process-local. It does not claim full conformance with every WARC profile, revisit/deduplication record type, CDX index, crawl log, or distributed archive deployment.

## Native capabilities

- Capture a normalized HTTP response as a WARC/1.1 response record.
- Validate canonical UUID record IDs, UTC WARC dates, HTTP(S) target URLs, status codes, and bounded content types.
- Remove URL fragments from archive target identity.
- Export retained records as concatenated WARC byte records.
- Retrieve records by record ID or latest normalized target URI.
- Enforce maximum record count, per-record bytes, and total retained bytes.
- Evict oldest records when configured retention capacity is reached.
- Report retained bytes, records captured, and eviction counts.
- Filter stored response headers through a small allowlist; sensitive headers such as Set-Cookie are not retained.

## Resource and data semantics

- Archive state is in-memory and process-local; it is not durable across restart.
- Captured response bodies are stored as supplied. Hosts must apply their own access control, retention schedule, encryption-at-rest, and data classification before exposing an archive.
- Each capture is bounded by configured serialized record and total archive limits.
- Duplicate record IDs within the retained archive are rejected.
- Export order is oldest to newest; query results are newest first.
- WARC dates currently accept canonical UTC second precision (`YYYY-MM-DDTHH:MM:SSZ`).

## Integration boundaries

- The archive engine does not perform network requests, crawl websites, or decide whether a URL may be fetched.
- Fetcher and Policy Engine remain responsible for destination validation, DNS checks, redirects, and URL policy.
- Crawler remains responsible for robots rules, scope, scheduling, and politeness.
- M22 archives a response supplied by a caller; it does not automatically capture every Fetcher response yet.

## Future work

- Optional durable archive backend through the Storage Engine.
- WARC revisit records, digest-based deduplication, and CDX/CDXJ indexing.
- Capture hooks at approved Fetcher/Crawler boundaries.
- Archive retention policy and access-control integration.
