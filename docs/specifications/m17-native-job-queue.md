# M17 — Native Job Queue & Processing Lifecycle

**Status:** Initial native queue integrated into the Rust runtime  
**Engine ID:** `engine.vrtc.queue`  
**Capability:** `queue.jobs`

## Research basis

M17 adopts the common lifecycle patterns used by task queues and distributed job systems:

- Priority-aware pending queues.
- Worker leases with expiration and ownership tokens.
- Explicit acknowledgement.
- Retry scheduling with caller-supplied delay.
- Attempt budgets and dead-letter routing.
- Recovery of expired leases after worker or runtime failure.
- Durable job records through the VRTc Storage Engine.

The implementation is deliberately a VRTc-owned queue contract, not a wrapper around a hosted broker.

## Native job model

Each job record contains:
- Stable job ID and job type.
- Opaque payload bytes.
- Priority and creation timestamp.
- Lifecycle status.
- Attempt count and maximum attempts.
- Not-before timestamp.
- Worker identity, lease expiration, and monotonically increasing lease generation.
- Completion timestamp and bounded last-error summary.

## Lifecycle

```text
Submit
  |
  v
Pending -- lease(worker, generation, expiry) --> Leased
  |                                               |
  |                                           +---+---+
  |                                           |       |
  |                                         Ack     Retry
  |                                           |       |
  |                                           v       v
  |                                       Completed Pending
  |                                                   |
  |                                           attempts exhausted
  |                                                   |
  +----------------------------------------------> DeadLetter

Expired lease -> recovery -> Pending or DeadLetter
Pending/Leased -> administrative Cancel -> Cancelled
```

## Delivery and ownership semantics

- Delivery is **at-least-once**, not exactly-once.
- A lease is bound to worker ID and generation. Stale workers cannot acknowledge or retry a later lease generation.
- A lease that has expired cannot be acknowledged; recovery must first move it back to Pending or DeadLetter.
- Jobs are selected by descending priority, then oldest creation time, then stable ID.
- Expired leases are recovered automatically during runtime initialization and can also be recovered explicitly.
- Attempt budgets count both explicit retries and expired leases.
- The caller supplies retry delay; exponential backoff and jitter policy belong to the scheduler/worker layer.
- Dead-letter jobs remain inspectable through a bounded query. Requeue/dead-letter replay is a future operation.

## Persistence

- Runtime always provides the shared Storage Engine to the Queue.
- With the default memory storage backend, queue state is ephemeral.
- With `RuntimeConfig.storage_path`, queue records are stored in the local redb database and loaded on startup.
- Each state transition persists the job record before updating the in-memory queue view.
- Expired-lease recovery writes all affected records in one storage batch.
- This is local persistence, not distributed coordination: multiple independent runtimes do not share one atomic in-memory lease state.

## Resource bounds

- Maximum jobs.
- Maximum payload bytes, job ID/type bytes, and worker ID bytes.
- Default and per-job attempt limits.
- Maximum lease duration and retry delay.
- Dead-letter listing limit.
- Persisted queue loading is bounded by the configured queue capacity and Storage Engine scan limit.

## Runtime integration

The runtime registers `engine.vrtc.queue` / `queue.jobs`, exposes `queue_engine()`, and uses the shared Storage Engine. Startup recovery runs before the runtime is returned to callers.

## Boundaries

- No worker executor or worker pool is included in M17; consumers lease, process, then ack/retry.
- No distributed broker or cross-process consensus is included.
- No NATS, RabbitMQ, Kafka, or Redis Streams adapter is mandatory.
- No cron scheduler, delayed-job calendar, rate limiter, or exactly-once guarantee.
- Payloads are opaque bytes; applications own serialization, encryption, and schema versioning.
- redb remains an experimental/beta embedded backend upstream.
