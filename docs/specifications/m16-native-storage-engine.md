# M16 — Native Storage & Database Engine

**Status:** Initial native engine integrated into the Rust runtime  
**Engine ID:** `engine.vrtc.storage`  
**Capability:** `storage.kv`

## Research basis and adoption

M16 uses a backend-neutral key-value contract informed by embedded database systems and transactional storage engines.

- **redb 2.6.4** is adopted as an optional embedded Rust backend. Its upstream documentation describes a pure-Rust, ACID transactional key-value database with copy-on-write B-trees, MVCC readers, and crash-safe commits. Its upstream status also labels the project beta; VRTc therefore treats it as an experimental embedded backend rather than claiming production maturity.
- **In-memory backend** remains the default for tests, ephemeral runs, and development.
- VRTc owns namespace isolation, key validation, size limits, batch semantics, and the backend trait. Application-facing contracts do not expose redb table definitions.

## Native storage contract

- Namespaced keys use a canonical `namespace/key` representation.
- Namespaces are limited to lowercase ASCII letters, digits, dots, underscores, and hyphens.
- Keys must be non-empty, bounded, and free of control characters.
- Values are opaque bytes with a configurable maximum size.
- Namespace scans are bounded and return entries in backend key order.
- Batch operations are validated before execution and applied atomically by the selected backend.
- Put and delete operations can be mixed in one batch.
- The backend abstraction is private to the crate; new adapters must implement the same transaction contract.

## Backends

### Memory
- Default runtime backend when no persistent path is configured.
- Thread-safe in-process BTreeMap.
- Batch updates are applied under one write lock.
- All data is lost when the process stops.

### redb
- Optional local file backend selected by `RuntimeConfig.storage_path`.
- Opens an existing database file or creates a new one.
- Parent directories are created when needed.
- Each batch executes in one redb write transaction and commits atomically.
- Reads use independent read transactions.
- The storage engine does not silently migrate incompatible redb file formats.

## Runtime configuration

- `RuntimeConfig.storage` controls key/value/batch/scan bounds.
- `RuntimeConfig.storage_path = None` selects memory.
- `RuntimeConfig.storage_path = Some(path)` selects the local redb backend.
- `storage_engine()` exposes the shared runtime handle.

## Important persistence boundary

The storage engine is now available as a shared runtime subsystem, but the current Delta, Evidence, Feedback, Frontier, and other engine state stores have **not yet been migrated to this backend**. They continue to use their own in-memory state until explicit serializers, schema versions, migrations, and recovery semantics are implemented. Do not interpret the presence of a redb file as proof that all VRTc state is durable.

## Security and operational boundaries

- No encryption at rest is implemented.
- No backup/restore, compaction scheduler, replication, remote object storage, or schema migration framework is implemented.
- Callers must protect the database file and parent directory with appropriate filesystem permissions.
- Values are opaque bytes; applications own serialization, schema versioning, and data minimization.
- The redb upstream project describes itself as beta quality; production deployment requires workload-specific durability, recovery, and compatibility evaluation.

## Future adapters

The backend contract is intended to support optional SQLite/PostgreSQL, RocksDB, object-store, or distributed adapters later. None is mandatory for the current runtime.
