use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram, MeterProvider as _};
use opentelemetry_otlp::{Protocol, WithExportConfig};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};

use crate::audit::Decision;
use crate::error::{Error, Result};

pub const EXPORT_INTERVAL: Duration = Duration::from_secs(30);
pub const EXPORT_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Metrics {
    provider: SdkMeterProvider,
    calls: Counter<u64>,
    refusals: Counter<u64>,
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

    pub fn start(endpoint: &str) -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();
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
            .with_description("Refused tool calls by rule")
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
        tracing::info!(endpoint, "metrics export is on");
        Ok(Self {
            provider,
            calls,
            refusals,
            duration,
            open_handles,
        })
    }

    pub fn record_call(
        &self,
        tool: &str,
        decision: Decision,
        rule: Option<&str>,
        duration: Duration,
    ) {
        let attributes = [
            KeyValue::new("tool", tool.to_owned()),
            KeyValue::new("decision", decision.as_str()),
        ];
        self.calls.add(1, &attributes);
        self.duration.record(
            duration.as_secs_f64() * 1_000.0,
            &[KeyValue::new("tool", tool.to_owned())],
        );
        if decision == Decision::Refused {
            let rule = rule
                .map(|rule| rule.chars().take(80).collect::<String>())
                .unwrap_or_else(|| "unknown".to_owned());
            self.refusals.add(1, &[KeyValue::new("rule", rule)]);
        }
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
