# M19 — Native Monitoring & Observability Engine

**Status:** Initial bounded in-process telemetry engine integrated into the Rust runtime  
**Engine ID:** `engine.vrtc.observability`  
**Capability:** `observability.telemetry`

## Research basis

M19 defines a VRTc-owned telemetry contract rather than binding the runtime to one vendor. It follows common observability separation:
- metrics for numeric operational measurements;
- structured events for discrete lifecycle and failure facts;
- bounded labels/cardinality to avoid unbounded memory growth;
- explicit payload minimization to reduce accidental secret and personal-data retention.

This initial crate does not claim full OpenTelemetry protocol compatibility or distributed tracing. Exporters and trace/span propagation can be added behind stable VRTc contracts after a separate compatibility and license review.

## Native capabilities

- Counters with overflow protection.
- Finite-value gauges.
- Cumulative histogram buckets with validated, strictly increasing boundaries.
- Bounded metric series and label dimensions.
- Bounded rolling operational event buffer with dropped-event accounting.
- Read-only metric snapshots, recent-event retrieval, and aggregate stats.
- Metric type conflict detection for a stable metric name/label key.

## Privacy and safety boundary

Operational events contain component ID, bounded event code, severity, and timestamp only. The engine intentionally has no arbitrary message or raw request payload field. Callers must not encode credentials, tokens, personal data, full URLs with query secrets, or user content into metric labels or event codes.

## Persistence and export boundary

- Initial storage is in-memory and bounded.
- No durable persistence, remote exporter, OpenTelemetry SDK integration, alert manager, dashboard, or distributed trace collector is included yet.
- Metrics and events are process-local; they are not a cross-process consensus or global monitoring service.

## Runtime integration

The runtime statically registers `engine.vrtc.observability` / `observability.telemetry` and exposes `observability_engine()`.

## Future work

- Optional OpenTelemetry-compatible metrics/traces exporter.
- Durable event persistence through the Storage Engine.
- Runtime instrumentation for queue, crawler, fetcher, search, and API operations.
- Health aggregation and alert rules with bounded cardinality.
