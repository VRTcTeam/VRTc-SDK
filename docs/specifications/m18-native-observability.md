# M18 — Native Observability Engine

**Status:** Initial bounded in-process telemetry engine integrated into the Rust runtime  
**Engine ID:** `engine.vrtc.observability`  
**Capability:** `observability.telemetry`

## Purpose

M18 gives VRTc-owned engines a shared telemetry contract for operational events, metric series, and health snapshots. It is a native runtime subsystem, not a hosted monitoring connector.

## Native capabilities

- Record structured events with component identity, event type, severity, timestamp, and bounded attributes.
- Retain a bounded event window; oldest events are evicted when capacity is reached.
- Maintain labeled counters and gauges with bounded metric cardinality.
- Prevent a metric identity from changing between counter and gauge kinds.
- Reject invalid identifiers, control characters, non-finite gauge values, and capacity overflow.
- Produce aggregate health snapshots across retained events and metric series.
- Return newest events first for bounded inspection.

## Data and resource semantics

- Event and metric state is in-memory and process-local.
- Event retention is a bounded ring-like queue; it is not an audit log or durable event store.
- Metric series are bounded by configured cardinality. Labels must be low-cardinality; callers must not use user IDs, URLs, tokens, or other unbounded/high-sensitivity values as labels.
- Counter values are monotonic within the lifetime of the runtime; reset/persistence semantics are not provided.
- Timestamps are caller-supplied Unix epoch milliseconds. The engine does not infer wall-clock time.
- Event attributes and metric labels are metadata only. Secrets and personal data must not be recorded.

## Runtime integration

The Rust runtime statically registers `engine.vrtc.observability` / `observability.telemetry` and exposes a shared `observability_engine()` handle. Engines and host applications may emit telemetry through that handle.

## Boundaries

- No OpenTelemetry SDK/exporter, Prometheus endpoint, OTLP transport, dashboard, alert manager, distributed trace context, or persistent telemetry backend is included in M18.
- The current implementation does not automatically instrument every engine; instrumentation must be added at meaningful execution boundaries.
- Health snapshots are operational summaries, not a full readiness/liveness policy.
- A future exporter can translate these VRTc-native records into OpenTelemetry/Prometheus formats without making either protocol a mandatory runtime dependency.
