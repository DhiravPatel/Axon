//! Process metrics endpoint (§41).
//!
//! `GET /metrics` returns Prometheus-style plaintext metrics so any
//! standard scraper can monitor an Axon serve deployment without a
//! custom exporter. Counters are atomic and process-wide; the registry
//! is exposed through [`MetricsRegistry`] which the server holds in an
//! `Arc`.
//!
//! Production deployments should still wrap this with TLS + auth — the
//! body is plaintext and intended for an internal scraping subnet.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug)]
pub struct MetricsRegistry {
    pub requests_total: AtomicU64,
    pub requests_success: AtomicU64,
    pub requests_error: AtomicU64,
    /// Cumulative wall time (microseconds) handlers have run.
    pub handler_us_total: AtomicU64,
    pub started: Instant,
    pub in_flight: AtomicU64,
    /// Total bytes returned to clients.
    pub bytes_out: AtomicU64,
    /// Total bytes read from clients.
    pub bytes_in: AtomicU64,
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self {
            requests_total: AtomicU64::new(0),
            requests_success: AtomicU64::new(0),
            requests_error: AtomicU64::new(0),
            handler_us_total: AtomicU64::new(0),
            started: Instant::now(),
            in_flight: AtomicU64::new(0),
            bytes_out: AtomicU64::new(0),
            bytes_in: AtomicU64::new(0),
        }
    }
}

impl MetricsRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn record_request(&self, status: u16, body_in: u64, body_out: u64, dur_us: u64) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        if (200..400).contains(&status) {
            self.requests_success.fetch_add(1, Ordering::Relaxed);
        } else {
            self.requests_error.fetch_add(1, Ordering::Relaxed);
        }
        self.bytes_in.fetch_add(body_in, Ordering::Relaxed);
        self.bytes_out.fetch_add(body_out, Ordering::Relaxed);
        self.handler_us_total.fetch_add(dur_us, Ordering::Relaxed);
    }

    /// Render a Prometheus 0.0.4 plaintext body.
    pub fn render_prometheus(&self) -> String {
        let total = self.requests_total.load(Ordering::Relaxed);
        let success = self.requests_success.load(Ordering::Relaxed);
        let error = self.requests_error.load(Ordering::Relaxed);
        let bytes_in = self.bytes_in.load(Ordering::Relaxed);
        let bytes_out = self.bytes_out.load(Ordering::Relaxed);
        let us = self.handler_us_total.load(Ordering::Relaxed);
        let uptime = self.started.elapsed().as_secs();
        let in_flight = self.in_flight.load(Ordering::Relaxed);
        let mut out = String::new();
        out.push_str("# HELP axon_uptime_seconds Process uptime in seconds.\n");
        out.push_str("# TYPE axon_uptime_seconds counter\n");
        out.push_str(&format!("axon_uptime_seconds {uptime}\n"));

        out.push_str("# HELP axon_requests_total Total HTTP requests handled.\n");
        out.push_str("# TYPE axon_requests_total counter\n");
        out.push_str(&format!("axon_requests_total {total}\n"));

        out.push_str("# HELP axon_requests_success_total HTTP requests with 2xx/3xx status.\n");
        out.push_str("# TYPE axon_requests_success_total counter\n");
        out.push_str(&format!("axon_requests_success_total {success}\n"));

        out.push_str("# HELP axon_requests_error_total HTTP requests with 4xx/5xx status.\n");
        out.push_str("# TYPE axon_requests_error_total counter\n");
        out.push_str(&format!("axon_requests_error_total {error}\n"));

        out.push_str("# HELP axon_in_flight_requests Currently-executing handlers.\n");
        out.push_str("# TYPE axon_in_flight_requests gauge\n");
        out.push_str(&format!("axon_in_flight_requests {in_flight}\n"));

        out.push_str("# HELP axon_bytes_in_total Total request body bytes read.\n");
        out.push_str("# TYPE axon_bytes_in_total counter\n");
        out.push_str(&format!("axon_bytes_in_total {bytes_in}\n"));

        out.push_str("# HELP axon_bytes_out_total Total response body bytes written.\n");
        out.push_str("# TYPE axon_bytes_out_total counter\n");
        out.push_str(&format!("axon_bytes_out_total {bytes_out}\n"));

        out.push_str("# HELP axon_handler_microseconds_total Cumulative handler execution time.\n");
        out.push_str("# TYPE axon_handler_microseconds_total counter\n");
        out.push_str(&format!("axon_handler_microseconds_total {us}\n"));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_split_by_success_and_error() {
        let m = MetricsRegistry::new();
        m.record_request(200, 100, 200, 500);
        m.record_request(500, 50, 80, 100);
        m.record_request(204, 0, 0, 200);
        assert_eq!(m.requests_total.load(Ordering::Relaxed), 3);
        assert_eq!(m.requests_success.load(Ordering::Relaxed), 2);
        assert_eq!(m.requests_error.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn prometheus_body_lists_every_metric() {
        let m = MetricsRegistry::new();
        m.record_request(200, 1, 2, 3);
        let body = m.render_prometheus();
        for needle in [
            "axon_uptime_seconds",
            "axon_requests_total 1",
            "axon_requests_success_total 1",
            "axon_requests_error_total 0",
            "axon_in_flight_requests",
            "axon_bytes_in_total 1",
            "axon_bytes_out_total 2",
            "axon_handler_microseconds_total 3",
        ] {
            assert!(body.contains(needle), "missing `{needle}` in:\n{body}");
        }
    }

    #[test]
    fn body_has_proper_help_and_type_lines() {
        let body = MetricsRegistry::new().render_prometheus();
        let help_count = body.matches("# HELP").count();
        let type_count = body.matches("# TYPE").count();
        assert_eq!(help_count, type_count);
        assert!(help_count >= 7);
    }
}
