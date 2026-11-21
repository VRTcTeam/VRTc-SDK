//! Native bounded metrics and operational event observability for VRTc.
//!
//! This first implementation is an in-process telemetry store. It deliberately
//! avoids raw request payloads and arbitrary event messages to reduce the risk
//! of storing credentials or personal data in operational telemetry.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, MutexGuard};

use vrtc_core::{ErrorCategory, VrtCError};

#[derive(Debug, Clone)]
pub struct ObservabilityConfig {
    pub max_metrics: usize,
    pub max_events: usize,
    pub max_name_bytes: usize,
    pub max_labels_per_metric: usize,
    pub max_label_bytes: usize,
    pub max_event_code_bytes: usize,
    pub histogram_buckets: Vec<f64>,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            max_metrics: 2_000,
            max_events: 5_000,
            max_name_bytes: 128,
            max_labels_per_metric: 8,
            max_label_bytes: 128,
            max_event_code_bytes: 96,
            histogram_buckets: vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MetricKey {
    pub name: String,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MetricValue {
    Counter(u64),
    Gauge(f64),
    Histogram {
        count: u64,
        sum: f64,
        cumulative_buckets: Vec<u64>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetricSnapshot {
    pub key: MetricKey,
    pub value: MetricValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventSeverity {
    Debug,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationalEvent {
    pub component_id: String,
    pub code: String,
    pub severity: EventSeverity,
    pub occurred_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ObservabilityStats {
    pub metric_series: usize,
    pub events_retained: usize,
    pub events_dropped: u64,
}

#[derive(Debug, Default)]
struct ObservabilityState {
    metrics: BTreeMap<MetricKey, MetricValue>,
    events: VecDeque<OperationalEvent>,
    events_dropped: u64,
}

#[derive(Debug)]
pub struct ObservabilityEngine {
    config: ObservabilityConfig,
    state: Mutex<ObservabilityState>,
}

impl ObservabilityEngine {
    pub fn new(config: ObservabilityConfig) -> Result<Self, VrtCError> {
        if config.max_metrics == 0
            || config.max_events == 0
            || config.max_name_bytes == 0
            || config.max_label_bytes == 0
            || config.max_event_code_bytes == 0
            || config.histogram_buckets.is_empty()
            || config.histogram_buckets.len() > 64
            || config.histogram_buckets
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0)
            || config.histogram_buckets.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(observability_error(
                "INVALID_OBSERVABILITY_CONFIG",
                "Telemetry limits and 1-64 strictly increasing positive histogram buckets are required",
                ErrorCategory::Validation,
            ));
        }
        Ok(Self {
            config,
            state: Mutex::new(ObservabilityState::default()),
        })
    }

    pub fn increment_counter(
        &self,
        name: &str,
        labels: BTreeMap<String, String>,
        amount: u64,
    ) -> Result<u64, VrtCError> {
        let key = self.metric_key(name, labels)?;
        let mut state = self.lock_state()?;
        self.ensure_capacity(&state, &key)?;
        match state.metrics.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(MetricValue::Counter(amount));
                Ok(amount)
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                match entry.get_mut() {
                    MetricValue::Counter(value) => {
                        *value = value.checked_add(amount).ok_or_else(|| {
                            observability_error(
                                "COUNTER_OVERFLOW",
                                "Counter increment exceeds the supported range",
                                ErrorCategory::ResourceExhausted,
                            )
                        })?;
                        Ok(*value)
                    }
                    _ => Err(observability_error(
                        "METRIC_TYPE_CONFLICT",
                        "Metric name and labels are already registered with another type",
                        ErrorCategory::ContractIncompatible,
                    )),
                }
            }
        }
    }

    pub fn set_gauge(
        &self,
        name: &str,
        labels: BTreeMap<String, String>,
        value: f64,
    ) -> Result<(), VrtCError> {
        if !value.is_finite() {
            return Err(observability_error(
                "INVALID_GAUGE_VALUE",
                "Gauge values must be finite",
                ErrorCategory::Validation,
            ));
        }
        let key = self.metric_key(name, labels)?;
        let mut state = self.lock_state()?;
        self.ensure_capacity(&state, &key)?;
        match state.metrics.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(MetricValue::Gauge(value));
                Ok(())
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => match entry.get_mut() {
                MetricValue::Gauge(current) => {
                    *current = value;
                    Ok(())
                }
                _ => Err(observability_error(
                    "METRIC_TYPE_CONFLICT",
                    "Metric name and labels are already registered with another type",
                    ErrorCategory::ContractIncompatible,
                )),
            },
        }
    }

    pub fn observe(
        &self,
        name: &str,
        labels: BTreeMap<String, String>,
        observation: f64,
    ) -> Result<(), VrtCError> {
        if !observation.is_finite() || observation < 0.0 {
            return Err(observability_error(
                "INVALID_HISTOGRAM_OBSERVATION",
                "Histogram observations must be finite and non-negative",
                ErrorCategory::Validation,
            ));
        }
        let key = self.metric_key(name, labels)?;
        let mut state = self.lock_state()?;
        self.ensure_capacity(&state, &key)?;
        let bucket_count = self.config.histogram_buckets.len();
        match state.metrics.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                let mut cumulative_buckets = vec![0; bucket_count];
                for (index, boundary) in self.config.histogram_buckets.iter().enumerate() {
                    if observation <= *boundary {
                        cumulative_buckets[index] = 1;
                    }
                }
                entry.insert(MetricValue::Histogram {
                    count: 1,
                    sum: observation,
                    cumulative_buckets,
                });
                Ok(())
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                match entry.get_mut() {
                    MetricValue::Histogram {
                        count,
                        sum,
                        cumulative_buckets,
                    } => {
                        let next_count = count.checked_add(1).ok_or_else(|| {
                            observability_error(
                                "HISTOGRAM_COUNT_OVERFLOW",
                                "Histogram observation count exceeds the supported range",
                                ErrorCategory::ResourceExhausted,
                            )
                        })?;
                        let next_sum = *sum + observation;
                        if !next_sum.is_finite() {
                            return Err(observability_error(
                                "HISTOGRAM_SUM_OVERFLOW",
                                "Histogram sum exceeds the supported range",
                                ErrorCategory::ResourceExhausted,
                            ));
                        }
                        *count = next_count;
                        *sum = next_sum;
                        for (index, boundary) in self.config.histogram_buckets.iter().enumerate() {
                            if observation <= *boundary {
                                cumulative_buckets[index] += 1;
                            }
                        }
                        Ok(())
                    }
                    _ => Err(observability_error(
                        "METRIC_TYPE_CONFLICT",
                        "Metric name and labels are already registered with another type",
                        ErrorCategory::ContractIncompatible,
                    )),
                }
            }
        }
    }

    pub fn record_event(&self, event: OperationalEvent) -> Result<(), VrtCError> {
        if event.component_id.trim().is_empty()
            || event.component_id.len() > self.config.max_name_bytes
            || event.code.trim().is_empty()
            || event.component_id.chars().any(char::is_control)
            || event.code.chars().any(char::is_control)
            || event.code.len() > self.config.max_event_code_bytes
        {
            return Err(observability_error(
                "INVALID_OPERATIONAL_EVENT",
                "Operational event component and code must be valid and bounded",
                ErrorCategory::Validation,
            ));
        }
        let mut state = self.lock_state()?;
        if state.events.len() == self.config.max_events {
            state.events.pop_front();
            state.events_dropped = state.events_dropped.saturating_add(1);
        }
        state.events.push_back(event);
        Ok(())
    }

    pub fn snapshot(&self) -> Result<Vec<MetricSnapshot>, VrtCError> {
        let state = self.lock_state()?;
        Ok(state
            .metrics
            .iter()
            .map(|(key, value)| MetricSnapshot {
                key: key.clone(),
                value: value.clone(),
            })
            .collect())
    }

    pub fn events(&self, limit: usize) -> Result<Vec<OperationalEvent>, VrtCError> {
        let state = self.lock_state()?;
        let bounded = limit.min(self.config.max_events);
        Ok(state.events.iter().rev().take(bounded).cloned().collect())
    }

    pub fn stats(&self) -> Result<ObservabilityStats, VrtCError> {
        let state = self.lock_state()?;
        Ok(ObservabilityStats {
            metric_series: state.metrics.len(),
            events_retained: state.events.len(),
            events_dropped: state.events_dropped,
        })
    }

    fn metric_key(
        &self,
        name: &str,
        labels: BTreeMap<String, String>,
    ) -> Result<MetricKey, VrtCError> {
        if name.trim().is_empty()
            || name.len() > self.config.max_name_bytes
            || name.chars().any(char::is_control)
            || labels.len() > self.config.max_labels_per_metric
            || labels.iter().any(|(key, value)| {
                key.trim().is_empty()
                    || value.trim().is_empty()
                    || key.len() > self.config.max_label_bytes
                    || value.len() > self.config.max_label_bytes
                    || key.chars().any(char::is_control)
                    || value.chars().any(char::is_control)
            })
        {
            return Err(observability_error(
                "INVALID_METRIC_KEY",
                "Metric name and bounded labels must be valid",
                ErrorCategory::Validation,
            ));
        }
        Ok(MetricKey {
            name: name.to_owned(),
            labels,
        })
    }

    fn ensure_capacity(
        &self,
        state: &ObservabilityState,
        key: &MetricKey,
    ) -> Result<(), VrtCError> {
        if !state.metrics.contains_key(key) && state.metrics.len() >= self.config.max_metrics {
            return Err(observability_error(
                "METRIC_CAPACITY_REACHED",
                "The configured metric-series capacity has been reached",
                ErrorCategory::ResourceExhausted,
            ));
        }
        Ok(())
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, ObservabilityState>, VrtCError> {
        self.state.lock().map_err(|_| {
            observability_error(
                "OBSERVABILITY_LOCK_POISONED",
                "The telemetry state lock is unavailable",
                ErrorCategory::Internal,
            )
        })
    }
}

fn observability_error(code: &str, message: &str, category: ErrorCategory) -> VrtCError {
    VrtCError::new(code, message, category, false).for_component("engine.vrtc.observability")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_gauges_and_histograms_are_recorded() {
        let engine = ObservabilityEngine::new(ObservabilityConfig::default()).unwrap();
        assert_eq!(
            engine
                .increment_counter("requests_total", BTreeMap::new(), 2)
                .unwrap(),
            2
        );
        engine
            .set_gauge("workers_active", BTreeMap::new(), 3.0)
            .unwrap();
        engine
            .observe("request_seconds", BTreeMap::new(), 0.02)
            .unwrap();
        assert_eq!(engine.snapshot().unwrap().len(), 3);
    }

    #[test]
    fn metric_type_conflicts_are_rejected() {
        let engine = ObservabilityEngine::new(ObservabilityConfig::default()).unwrap();
        engine
            .increment_counter("requests_total", BTreeMap::new(), 1)
            .unwrap();
        assert_eq!(
            engine.set_gauge("requests_total", BTreeMap::new(), 1.0).unwrap_err().code,
            "METRIC_TYPE_CONFLICT"
        );
    }

    #[test]
    fn event_buffer_is_bounded_and_reports_drops() {
        let config = ObservabilityConfig {
            max_events: 1,
            ..ObservabilityConfig::default()
        };
        let engine = ObservabilityEngine::new(config).unwrap();
        for code in ["FIRST", "SECOND"] {
            engine
                .record_event(OperationalEvent {
                    component_id: "engine.vrtc.search".to_owned(),
                    code: code.to_owned(),
                    severity: EventSeverity::Warning,
                    occurred_at_ms: 1,
                })
                .unwrap();
        }
        assert_eq!(engine.stats().unwrap().events_dropped, 1);
        assert_eq!(engine.events(10).unwrap()[0].code, "SECOND");
    }

    #[test]
    fn rejects_non_finite_values_and_invalid_histogram_buckets() {
        let engine = ObservabilityEngine::new(ObservabilityConfig::default()).unwrap();
        assert_eq!(
            engine
                .set_gauge("load", BTreeMap::new(), f64::NAN)
                .unwrap_err()
                .code,
            "INVALID_GAUGE_VALUE"
        );
        let config = ObservabilityConfig {
            histogram_buckets: vec![1.0, 0.5],
            ..ObservabilityConfig::default()
        };
        assert_eq!(
            ObservabilityEngine::new(config).unwrap_err().code,
            "INVALID_OBSERVABILITY_CONFIG"
        );
    }
}
