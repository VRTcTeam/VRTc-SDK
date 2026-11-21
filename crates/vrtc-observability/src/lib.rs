//! Bounded, runtime-owned telemetry primitives for VRTc engines.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use vrtc_core::{ErrorCategory, VrtCError};

#[derive(Debug, Clone)]
pub struct ObservabilityConfig {
    pub max_events: usize,
    pub max_metrics: usize,
    pub max_attributes_per_event: usize,
    pub max_attribute_bytes: usize,
    pub max_metric_name_bytes: usize,
    pub max_labels_per_metric: usize,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            max_events: 4096,
            max_metrics: 1024,
            max_attributes_per_event: 16,
            max_attribute_bytes: 1024,
            max_metric_name_bytes: 128,
            max_labels_per_metric: 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryEvent {
    pub id: String,
    pub component_id: String,
    pub event_type: String,
    pub severity: Severity,
    pub timestamp_ms: u64,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricKind {
    Counter,
    Gauge,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricSnapshot {
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub kind: MetricKind,
    pub value: f64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HealthSnapshot {
    pub retained_events: usize,
    pub total_events_recorded: u64,
    pub metric_series: usize,
    pub trace_events: usize,
    pub debug_events: usize,
    pub info_events: usize,
    pub warn_events: usize,
    pub error_events: usize,
    pub latest_event_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MetricIdentity {
    name: String,
    labels: BTreeMap<String, String>,
}

#[derive(Default)]
struct State {
    events: VecDeque<TelemetryEvent>,
    metrics: BTreeMap<MetricIdentity, MetricSnapshot>,
    total_events_recorded: u64,
}

pub struct ObservabilityEngine {
    config: ObservabilityConfig,
    state: Mutex<State>,
}

impl ObservabilityEngine {
    pub fn new(config: ObservabilityConfig) -> Result<Self, VrtCError> {
        if config.max_events == 0
            || config.max_metrics == 0
            || config.max_attributes_per_event == 0
            || config.max_attribute_bytes == 0
            || config.max_metric_name_bytes == 0
        {
            return Err(error(
                "INVALID_OBSERVABILITY_CONFIG",
                "Observability capacity and size limits must be greater than zero",
                ErrorCategory::Validation,
            ));
        }
        Ok(Self {
            config,
            state: Mutex::new(State::default()),
        })
    }

    pub fn record_event(&self, event: TelemetryEvent) -> Result<(), VrtCError> {
        if !valid_identifier(&event.id, 256)
            || !valid_identifier(&event.component_id, 256)
            || !valid_identifier(&event.event_type, 128)
            || event.attributes.len() > self.config.max_attributes_per_event
            || event.attributes.iter().any(|(key, value)| {
                !valid_identifier(key, 128)
                    || key.len() > self.config.max_attribute_bytes
                    || value.len() > self.config.max_attribute_bytes
                    || value.chars().any(char::is_control)
            })
        {
            return Err(error(
                "INVALID_TELEMETRY_EVENT",
                "Event identifiers and bounded attributes must be valid",
                ErrorCategory::Validation,
            ));
        }

        let mut state = self.lock_state()?;
        if state.events.len() == self.config.max_events {
            state.events.pop_front();
        }
        state.total_events_recorded = state.total_events_recorded.saturating_add(1);
        state.events.push_back(event);
        Ok(())
    }

    pub fn increment_counter(
        &self,
        name: &str,
        labels: BTreeMap<String, String>,
        amount: u64,
        timestamp_ms: u64,
    ) -> Result<MetricSnapshot, VrtCError> {
        if amount == 0 {
            return Err(error(
                "INVALID_COUNTER_INCREMENT",
                "Counter increments must be greater than zero",
                ErrorCategory::Validation,
            ));
        }
        self.update_metric(name, labels, MetricKind::Counter, amount as f64, timestamp_ms, true)
    }

    pub fn set_gauge(
        &self,
        name: &str,
        labels: BTreeMap<String, String>,
        value: f64,
        timestamp_ms: u64,
    ) -> Result<MetricSnapshot, VrtCError> {
        if !value.is_finite() {
            return Err(error(
                "INVALID_GAUGE_VALUE",
                "Gauge values must be finite numbers",
                ErrorCategory::Validation,
            ));
        }
        self.update_metric(name, labels, MetricKind::Gauge, value, timestamp_ms, false)
    }

    pub fn events(&self, limit: usize) -> Result<Vec<TelemetryEvent>, VrtCError> {
        if limit > self.config.max_events {
            return Err(error(
                "OBSERVABILITY_QUERY_LIMIT",
                "Event query limit exceeds configured capacity",
                ErrorCategory::ResourceExhausted,
            ));
        }
        let state = self.lock_state()?;
        Ok(state.events.iter().rev().take(limit).cloned().collect())
    }

    pub fn metrics(&self) -> Result<Vec<MetricSnapshot>, VrtCError> {
        let state = self.lock_state()?;
        Ok(state.metrics.values().cloned().collect())
    }

    pub fn health_snapshot(&self) -> Result<HealthSnapshot, VrtCError> {
        let state = self.lock_state()?;
        let mut snapshot = HealthSnapshot {
            retained_events: state.events.len(),
            total_events_recorded: state.total_events_recorded,
            metric_series: state.metrics.len(),
            latest_event_at_ms: state.events.back().map(|event| event.timestamp_ms),
            ..HealthSnapshot::default()
        };
        for event in &state.events {
            match event.severity {
                Severity::Trace => snapshot.trace_events += 1,
                Severity::Debug => snapshot.debug_events += 1,
                Severity::Info => snapshot.info_events += 1,
                Severity::Warn => snapshot.warn_events += 1,
                Severity::Error => snapshot.error_events += 1,
            }
        }
        Ok(snapshot)
    }

    fn update_metric(
        &self,
        name: &str,
        labels: BTreeMap<String, String>,
        kind: MetricKind,
        value: f64,
        timestamp_ms: u64,
        additive: bool,
    ) -> Result<MetricSnapshot, VrtCError> {
        if !valid_identifier(name, self.config.max_metric_name_bytes)
            || labels.len() > self.config.max_labels_per_metric
            || labels.iter().any(|(key, value)| {
                !valid_identifier(key, 128)
                    || key.len() > self.config.max_attribute_bytes
                    || value.len() > self.config.max_attribute_bytes
                    || value.chars().any(char::is_control)
            })
        {
            return Err(error(
                "INVALID_METRIC_IDENTITY",
                "Metric name and bounded labels must be valid",
                ErrorCategory::Validation,
            ));
        }
        let identity = MetricIdentity {
            name: name.to_owned(),
            labels: labels.clone(),
        };
        let mut state = self.lock_state()?;
        let existing = state.metrics.get(&identity).cloned();
        if existing.is_none() && state.metrics.len() >= self.config.max_metrics {
            return Err(error(
                "METRIC_CARDINALITY_LIMIT",
                "Metric series capacity has been reached",
                ErrorCategory::ResourceExhausted,
            ));
        }
        if existing.as_ref().is_some_and(|metric| metric.kind != kind) {
            return Err(error(
                "METRIC_KIND_CONFLICT",
                "A metric identity cannot change its kind",
                ErrorCategory::Validation,
            ));
        }
        let next_value = if additive {
            existing.map_or(value, |metric| metric.value + value)
        } else {
            value
        };
        if !next_value.is_finite() {
            return Err(error(
                "METRIC_VALUE_OVERFLOW",
                "Metric update produced a non-finite value",
                ErrorCategory::ResourceExhausted,
            ));
        }
        let metric = MetricSnapshot {
            name: name.to_owned(),
            labels,
            kind,
            value: next_value,
            updated_at_ms: timestamp_ms,
        };
        state.metrics.insert(identity, metric.clone());
        Ok(metric)
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, State>, VrtCError> {
        self.state.lock().map_err(|_| {
            error(
                "OBSERVABILITY_LOCK_POISONED",
                "Observability state is temporarily unavailable",
                ErrorCategory::Internal,
            )
        })
    }
}

fn valid_identifier(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= max_bytes
        && !value.chars().any(char::is_control)
}

fn error(code: &str, message: &str, category: ErrorCategory) -> VrtCError {
    VrtCError::new(code, message, category, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str, severity: Severity) -> TelemetryEvent {
        TelemetryEvent {
            id: id.to_owned(),
            component_id: "engine.vrtc.search".to_owned(),
            event_type: "search.completed".to_owned(),
            severity,
            timestamp_ms: 10,
            attributes: BTreeMap::new(),
        }
    }

    #[test]
    fn event_buffer_is_bounded_and_health_is_aggregated() {
        let engine = ObservabilityEngine::new(ObservabilityConfig {
            max_events: 2,
            ..ObservabilityConfig::default()
        })
        .unwrap();
        engine.record_event(event("event-1", Severity::Info)).unwrap();
        engine.record_event(event("event-2", Severity::Warn)).unwrap();
        engine.record_event(event("event-3", Severity::Error)).unwrap();

        let snapshot = engine.health_snapshot().unwrap();
        assert_eq!(snapshot.total_events_recorded, 3);
        assert_eq!(snapshot.retained_events, 2);
        assert_eq!(snapshot.warn_events, 1);
        assert_eq!(snapshot.error_events, 1);
        assert_eq!(engine.events(2).unwrap()[0].id, "event-3");
    }

    #[test]
    fn counters_and_gauges_have_distinct_update_semantics() {
        let engine = ObservabilityEngine::new(ObservabilityConfig::default()).unwrap();
        let labels = BTreeMap::from([("engine".to_owned(), "search".to_owned())]);
        assert_eq!(
            engine.increment_counter("requests_total", labels.clone(), 2, 1).unwrap().value,
            2.0
        );
        assert_eq!(
            engine.increment_counter("requests_total", labels.clone(), 3, 2).unwrap().value,
            5.0
        );
        assert_eq!(
            engine.set_gauge("queue_depth", labels, 4.0, 3).unwrap().value,
            4.0
        );
    }

    #[test]
    fn metric_cardinality_and_invalid_values_are_rejected() {
        let engine = ObservabilityEngine::new(ObservabilityConfig {
            max_metrics: 1,
            ..ObservabilityConfig::default()
        })
        .unwrap();
        engine.set_gauge("queue_depth", BTreeMap::new(), 1.0, 1).unwrap();
        let error = engine
            .set_gauge("worker_count", BTreeMap::new(), 2.0, 2)
            .unwrap_err();
        assert_eq!(error.code, "METRIC_CARDINALITY_LIMIT");
        let error = engine
            .set_gauge("queue_depth", BTreeMap::new(), f64::NAN, 3)
            .unwrap_err();
        assert_eq!(error.code, "INVALID_GAUGE_VALUE");
    }
}
