use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram, MeterProvider as _};
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};

use crate::audit::{Decision, Sink};
use crate::engine::Engine;
use crate::error::{Error, Result};

pub const EXPORT_INTERVAL: Duration = Duration::from_secs(30);
pub const EXPORT_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Metrics {
    provider: SdkMeterProvider,
    calls: Counter<u64>,
    refusals: Counter<u64>,
    http_rejections: Counter<u64>,
    duration: Histogram<f64>,
    open_handles: Arc<AtomicU64>,
}

impl std::fmt::Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}

impl Metrics {
    #[must_use]
    pub fn metrics_url(endpoint: &str) -> String {
        let trimmed = endpoint.trim().trim_end_matches('/');
        if trimmed.ends_with("/v1/metrics") {
            trimmed.to_owned()
        } else {
            format!("{trimmed}/v1/metrics")
        }
    }

    pub fn start(endpoint: &str, audit: Arc<Sink>, engine: Arc<Engine>) -> Result<Self> {
        super::ensure_tls_provider();
        super::http::announce_proxy("metrics export");
        let exporter = opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(Self::metrics_url(endpoint))
            .with_timeout(EXPORT_TIMEOUT)
            .build()
            .map_err(|error| Error::ConfigInvalid {
                setting: "http.otel_endpoint".to_owned(),
                value: endpoint.to_owned(),
                detail: format!("the metrics exporter could not be built: {error}"),
            })?;
        let reader = PeriodicReader::builder(exporter)
            .with_interval(EXPORT_INTERVAL)
            .build();
        let resource = opentelemetry_sdk::Resource::builder()
            .with_service_name("ownpg")
            .with_attribute(KeyValue::new("service.version", crate::VERSION))
            .build();
        let provider = SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(resource)
            .build();
        let meter = provider.meter("ownpg");
        let calls = meter
            .u64_counter("ownpg.calls")
            .with_description("Tool calls by tool and decision")
            .build();
        let refusals = meter
            .u64_counter("ownpg.refusals")
            .with_description("Refused tool calls by error code")
            .build();
        let http_rejections = meter
            .u64_counter("ownpg.http.rejections")
            .with_description("HTTP requests refused before they reached a tool, by reason")
            .build();
        let duration = meter
            .f64_histogram("ownpg.call.duration")
            .with_description("Tool call duration")
            .with_unit("ms")
            .build();
        let open_handles = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&open_handles);
        let _gauge = meter
            .u64_observable_gauge("ownpg.open_handles")
            .with_description("Open cursor and transaction handles")
            .with_callback(move |observer| {
                observer.observe(observed.load(Ordering::Relaxed), &[]);
            })
            .build();
        let dropped_source = Arc::clone(&audit);
        let _dropped = meter
            .u64_observable_counter("ownpg.audit.dropped")
            .with_description("Audit lines that could not be written since the server started")
            .with_callback(move |observer| {
                observer.observe(dropped_source.dropped(), &[]);
            })
            .build();
        let _degraded = meter
            .u64_observable_gauge("ownpg.audit.degraded")
            .with_description("1 while the audit log cannot be written, otherwise 0")
            .with_callback(move |observer| {
                observer.observe(
                    u64::from(audit.is_degraded()),
                    &[KeyValue::new("on_failure", audit.on_failure().as_str())],
                );
            })
            .build();
        Self::observe_pool(&meter, &engine);
        tracing::info!(endpoint, "metrics export is on");
        Ok(Self {
            provider,
            calls,
            refusals,
            http_rejections,
            duration,
            open_handles,
        })
    }

    fn observe_pool(meter: &opentelemetry::metrics::Meter, engine: &Arc<Engine>) {
        let pool_name = KeyValue::new(
            "db.client.connection.pool.name",
            engine.settings().database.value.clone(),
        );
        let source = Arc::clone(engine);
        let name = pool_name.clone();
        let _connections = meter
            .i64_observable_up_down_counter("db.client.connection.count")
            .with_description("Pooled database connections by state")
            .with_unit("{connection}")
            .with_callback(move |observer| {
                if let Some(status) = source.pool_status() {
                    for (state, value) in [("idle", status.idle), ("used", status.used)] {
                        observer.observe(
                            pool_count(value),
                            &[
                                name.clone(),
                                KeyValue::new("db.client.connection.state", state),
                            ],
                        );
                    }
                }
            })
            .build();
        let source = Arc::clone(engine);
        let name = pool_name.clone();
        let _max = meter
            .i64_observable_up_down_counter("db.client.connection.max")
            .with_description("The most pooled database connections allowed")
            .with_unit("{connection}")
            .with_callback(move |observer| {
                if let Some(status) = source.pool_status() {
                    observer.observe(pool_count(status.max), std::slice::from_ref(&name));
                }
            })
            .build();
        let source = Arc::clone(engine);
        let _pending = meter
            .i64_observable_up_down_counter("db.client.connection.pending_requests")
            .with_description("Calls waiting for a pooled database connection")
            .with_unit("{request}")
            .with_callback(move |observer| {
                if let Some(status) = source.pool_status() {
                    observer.observe(pool_count(status.waiting), std::slice::from_ref(&pool_name));
                }
            })
            .build();
    }

    pub fn record_call(
        &self,
        tool: &str,
        decision: Decision,
        error_code: Option<&str>,
        duration: Duration,
    ) {
        let outcome = if error_code.is_some() { "error" } else { "ok" };
        let attributes = [
            KeyValue::new("tool", tool.to_owned()),
            KeyValue::new("decision", decision.as_str()),
            KeyValue::new("outcome", outcome),
        ];
        self.calls.add(1, &attributes);
        self.duration.record(
            duration.as_secs_f64() * 1_000.0,
            &[
                KeyValue::new("tool", tool.to_owned()),
                KeyValue::new("outcome", outcome),
            ],
        );
        if decision == Decision::Refused {
            self.refusals.add(
                1,
                &[
                    KeyValue::new("tool", tool.to_owned()),
                    KeyValue::new("code", error_code.unwrap_or("unknown").to_owned()),
                ],
            );
        }
    }

    pub fn record_http_rejection(&self, reason: &'static str) {
        self.http_rejections
            .add(1, &[KeyValue::new("reason", reason)]);
    }

    #[must_use]
    pub fn open_handles_gauge(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.open_handles)
    }

    pub fn set_open_handles(&self, count: u64) {
        self.open_handles.store(count, Ordering::Relaxed);
    }

    pub async fn shutdown(&self) {
        let provider = self.provider.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            if let Err(error) = provider.force_flush() {
                tracing::debug!(%error, "metrics could not be flushed");
            }
            if let Err(error) = provider.shutdown() {
                tracing::debug!(%error, "the metrics provider did not shut down cleanly");
            }
        })
        .await;
        if let Err(error) = outcome {
            tracing::debug!(%error, "the metrics shutdown task ended abnormally");
        }
    }
}

fn pool_count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_metrics_path_is_appended_once() {
        assert_eq!(
            Metrics::metrics_url("http://collector:4318"),
            "http://collector:4318/v1/metrics"
        );
        assert_eq!(
            Metrics::metrics_url("http://collector:4318/"),
            "http://collector:4318/v1/metrics"
        );
        assert_eq!(
            Metrics::metrics_url("https://otel.example/v1/metrics"),
            "https://otel.example/v1/metrics"
        );
    }
}
