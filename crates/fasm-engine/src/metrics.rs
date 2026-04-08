//! Prometheus-style metrics registry.
//!
//! Tracks per-function invocation counts, durations, error counts, queue
//! depths, and active sandbox counts. Exposed at `GET /metrics` as plain text.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

// ── Snapshot types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct MetricsSnapshot {
    pub invocations: HashMap<String, u64>,
    pub errors: HashMap<String, u64>,
    pub durations_ms: HashMap<String, Vec<u64>>,
    pub queue_depth: HashMap<String, u64>,
    pub active_sandboxes: u64,
    pub dropped_total: HashMap<String, u64>,
    pub custom_counters: HashMap<String, i64>,
}

// ── Registry ───────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct Inner {
    invocations: HashMap<String, u64>,
    errors: HashMap<String, u64>,
    durations_ms: HashMap<String, Vec<u64>>,
    queue_depth: HashMap<String, u64>,
    active_sandboxes: u64,
    dropped_total: HashMap<String, u64>,
    custom_counters: HashMap<String, i64>,
}

#[derive(Debug, Clone, Default)]
pub struct MetricsRegistry(Arc<Mutex<Inner>>);

impl MetricsRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    // ── write ────────────────────────────────────────────────────────────

    pub fn record_invocation(&self, func: &str) {
        let mut g = self.0.lock().unwrap();
        *g.invocations.entry(func.to_string()).or_default() += 1;
    }

    pub fn record_error(&self, func: &str) {
        let mut g = self.0.lock().unwrap();
        *g.errors.entry(func.to_string()).or_default() += 1;
    }

    pub fn record_duration_ms(&self, func: &str, ms: u64) {
        let mut g = self.0.lock().unwrap();
        g.durations_ms.entry(func.to_string()).or_default().push(ms);
    }

    pub fn set_queue_depth(&self, queue: &str, depth: u64) {
        let mut g = self.0.lock().unwrap();
        g.queue_depth.insert(queue.to_string(), depth);
    }

    pub fn inc_active(&self) {
        self.0.lock().unwrap().active_sandboxes += 1;
    }

    pub fn dec_active(&self) {
        let mut g = self.0.lock().unwrap();
        g.active_sandboxes = g.active_sandboxes.saturating_sub(1);
    }

    pub fn record_dropped(&self, trigger: &str) {
        let mut g = self.0.lock().unwrap();
        *g.dropped_total.entry(trigger.to_string()).or_default() += 1;
    }

    pub fn custom_inc(&self, key: &str, delta: i64) {
        let mut g = self.0.lock().unwrap();
        *g.custom_counters.entry(key.to_string()).or_default() += delta;
    }

    pub fn custom_set(&self, key: &str, val: i64) {
        let mut g = self.0.lock().unwrap();
        g.custom_counters.insert(key.to_string(), val);
    }

    pub fn custom_get(&self, key: &str) -> Option<i64> {
        self.0.lock().unwrap().custom_counters.get(key).copied()
    }

    // ── read ─────────────────────────────────────────────────────────────

    pub fn snapshot(&self) -> MetricsSnapshot {
        let g = self.0.lock().unwrap();
        MetricsSnapshot {
            invocations: g.invocations.clone(),
            errors: g.errors.clone(),
            durations_ms: g.durations_ms.clone(),
            queue_depth: g.queue_depth.clone(),
            active_sandboxes: g.active_sandboxes,
            dropped_total: g.dropped_total.clone(),
            custom_counters: g.custom_counters.clone(),
        }
    }

    /// Render a Prometheus-compatible text exposition.
    pub fn render_text(&self) -> String {
        let snap = self.snapshot();
        let mut out = String::new();

        // Invocations
        out.push_str("# HELP fasm_invocations_total Total function invocations\n");
        out.push_str("# TYPE fasm_invocations_total counter\n");
        for (func, count) in &snap.invocations {
            out.push_str(&format!(
                "fasm_invocations_total{{function=\"{}\"}} {}\n",
                func, count
            ));
        }

        // Errors
        out.push_str("# HELP fasm_errors_total Total function errors\n");
        out.push_str("# TYPE fasm_errors_total counter\n");
        for (func, count) in &snap.errors {
            out.push_str(&format!(
                "fasm_errors_total{{function=\"{}\"}} {}\n",
                func, count
            ));
        }

        // Durations (p50, p99, count, sum)
        out.push_str("# HELP fasm_invocation_duration_ms Execution time histogram (ms)\n");
        out.push_str("# TYPE fasm_invocation_duration_ms summary\n");
        for (func, samples) in &snap.durations_ms {
            if samples.is_empty() {
                continue;
            }
            let mut sorted = samples.clone();
            sorted.sort_unstable();
            let count = sorted.len();
            let sum: u64 = sorted.iter().sum();
            let p50 = sorted[count / 2];
            let p99 = sorted[(count * 99 / 100).min(count - 1)];
            out.push_str(&format!(
                "fasm_invocation_duration_ms{{function=\"{}\",quantile=\"0.5\"}} {}\n",
                func, p50
            ));
            out.push_str(&format!(
                "fasm_invocation_duration_ms{{function=\"{}\",quantile=\"0.99\"}} {}\n",
                func, p99
            ));
            out.push_str(&format!(
                "fasm_invocation_duration_ms_count{{function=\"{}\"}} {}\n",
                func, count
            ));
            out.push_str(&format!(
                "fasm_invocation_duration_ms_sum{{function=\"{}\"}} {}\n",
                func, sum
            ));
        }

        // Queue depths
        out.push_str("# HELP fasm_queue_depth Messages waiting per named queue\n");
        out.push_str("# TYPE fasm_queue_depth gauge\n");
        for (q, depth) in &snap.queue_depth {
            out.push_str(&format!("fasm_queue_depth{{queue=\"{}\"}} {}\n", q, depth));
        }

        // Active sandboxes
        out.push_str("# HELP fasm_active_sandboxes Currently executing sandboxes\n");
        out.push_str("# TYPE fasm_active_sandboxes gauge\n");
        out.push_str(&format!(
            "fasm_active_sandboxes {}\n",
            snap.active_sandboxes
        ));

        // Dropped
        out.push_str("# HELP fasm_dropped_executions_total Dropped executions by trigger\n");
        out.push_str("# TYPE fasm_dropped_executions_total counter\n");
        for (trigger, count) in &snap.dropped_total {
            out.push_str(&format!(
                "fasm_dropped_executions_total{{trigger=\"{}\"}} {}\n",
                trigger, count
            ));
        }

        // Custom
        for (key, val) in &snap.custom_counters {
            out.push_str(&format!("fasm_custom{{key=\"{}\"}} {}\n", key, val));
        }

        out
    }
}

// ── Timer helper ─────────────────────────────────────────────────────────────

/// RAII guard that records a duration on drop.
pub struct Timer {
    started: Instant,
    func: String,
    metrics: MetricsRegistry,
}

impl Timer {
    pub fn start(func: impl Into<String>, metrics: MetricsRegistry) -> Self {
        Self {
            started: Instant::now(),
            func: func.into(),
            metrics,
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let ms = self.started.elapsed().as_millis() as u64;
        self.metrics.record_duration_ms(&self.func, ms);
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invocation_counter_increments() {
        let m = MetricsRegistry::new();
        m.record_invocation("Ping");
        m.record_invocation("Ping");
        m.record_invocation("Echo");
        let snap = m.snapshot();
        assert_eq!(snap.invocations["Ping"], 2);
        assert_eq!(snap.invocations["Echo"], 1);
    }

    #[test]
    fn test_error_counter() {
        let m = MetricsRegistry::new();
        m.record_error("Fib");
        m.record_error("Fib");
        let snap = m.snapshot();
        assert_eq!(snap.errors["Fib"], 2);
    }

    #[test]
    fn test_active_sandboxes_inc_dec() {
        let m = MetricsRegistry::new();
        m.inc_active();
        m.inc_active();
        m.inc_active();
        assert_eq!(m.snapshot().active_sandboxes, 3);
        m.dec_active();
        assert_eq!(m.snapshot().active_sandboxes, 2);
    }

    #[test]
    fn test_active_sandboxes_does_not_underflow() {
        let m = MetricsRegistry::new();
        m.dec_active(); // should not panic
        assert_eq!(m.snapshot().active_sandboxes, 0);
    }

    #[test]
    fn test_custom_inc_and_get() {
        let m = MetricsRegistry::new();
        m.custom_inc("my_counter", 5);
        m.custom_inc("my_counter", 3);
        assert_eq!(m.custom_get("my_counter"), Some(8));
        assert_eq!(m.custom_get("nonexistent"), None);
    }

    #[test]
    fn test_custom_set_overwrites() {
        let m = MetricsRegistry::new();
        m.custom_set("gauge", 100);
        m.custom_set("gauge", 42);
        assert_eq!(m.custom_get("gauge"), Some(42));
    }

    #[test]
    fn test_render_text_contains_required_headers() {
        let m = MetricsRegistry::new();
        m.record_invocation("Ping");
        m.set_queue_depth("orders", 7);
        let text = m.render_text();
        assert!(
            text.contains("fasm_invocations_total{function=\"Ping\"} 1"),
            "missing invocation line"
        );
        assert!(
            text.contains("fasm_queue_depth{queue=\"orders\"} 7"),
            "missing queue depth line"
        );
        assert!(
            text.contains("fasm_active_sandboxes"),
            "missing active sandboxes metric"
        );
    }

    #[test]
    fn test_dropped_counter() {
        let m = MetricsRegistry::new();
        m.record_dropped("http");
        m.record_dropped("http");
        m.record_dropped("schedule");
        let snap = m.snapshot();
        assert_eq!(snap.dropped_total["http"], 2);
        assert_eq!(snap.dropped_total["schedule"], 1);
    }
}
