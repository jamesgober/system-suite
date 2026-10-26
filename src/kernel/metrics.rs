//! The kernel's own metrics. Without the `metrics` feature every call is a
//! no-op.
//!
//! | Metric | Kind | Normal | Anomalous |
//! |---|---|---|---|
//! | `system_suite_boot_milliseconds` | gauge | a few ms | a slow start: see the boot timeline in the log |
//! | `system_suite_urgent_level` | gauge (0 normal to 4 stopping) | 0 | above 0 outside a stop: shedding or lockdown |
//! | `system_suite_worker_restarts_total{worker}` | counter | 0 | growing: a worker keeps failing |
//! | `system_suite_worker_failures_total{worker}` | counter | 0 | any: a worker exhausted its restart budget |
//! | `system_suite_subsystem_errors_total{subsystem}` | counter | low | growing: a subsystem reports errors it handled |
//! | `system_suite_reloads_total{outcome}` | counter | `accepted` grows with reloads | `refused` grows: a configuration change was rejected |

use super::Level;

/// The kernel's metrics, recorded into the system's registry.
#[derive(Clone)]
pub(crate) struct KernelMetrics {
    #[cfg(feature = "metrics")]
    core: Option<crate::metrics_handle::MetricsRef>,
}

impl KernelMetrics {
    /// No metrics: the `metrics` member is compiled out or disabled.
    pub(crate) fn none() -> KernelMetrics {
        KernelMetrics {
            #[cfg(feature = "metrics")]
            core: None,
        }
    }

    #[cfg(feature = "metrics")]
    pub(crate) fn new(core: crate::metrics_handle::MetricsRef) -> KernelMetrics {
        KernelMetrics { core: Some(core) }
    }

    pub(crate) fn boot_milliseconds(&self, ms: u64) {
        #[cfg(feature = "metrics")]
        if let Some(core) = &self.core {
            core.gauge("system_suite_boot_milliseconds").set(ms as f64);
        }
        #[cfg(not(feature = "metrics"))]
        let _unused = ms;
    }

    pub(crate) fn urgent(&self, level: Level) {
        #[cfg(feature = "metrics")]
        if let Some(core) = &self.core {
            core.gauge("system_suite_urgent_level")
                .set(f64::from(level as u8));
        }
        #[cfg(not(feature = "metrics"))]
        let _unused = level;
    }

    pub(crate) fn worker_restarted(&self, name: &str) {
        self.count("system_suite_worker_restarts_total", "worker", name);
    }

    pub(crate) fn worker_failed(&self, name: &str) {
        self.count("system_suite_worker_failures_total", "worker", name);
    }

    pub(crate) fn subsystem_error(&self, name: &str) {
        self.count("system_suite_subsystem_errors_total", "subsystem", name);
    }

    #[cfg_attr(not(feature = "config"), allow(dead_code))]
    pub(crate) fn reload(&self, accepted: bool) {
        let outcome = if accepted { "accepted" } else { "refused" };
        self.count("system_suite_reloads_total", "outcome", outcome);
    }

    fn count(&self, metric: &'static str, key: &'static str, value: &str) {
        #[cfg(feature = "metrics")]
        if let Some(core) = &self.core {
            let labels = metrics_lib::LabelSet::EMPTY.with(key, value.to_owned());
            core.counter_with(metric, &labels).inc();
        }
        #[cfg(not(feature = "metrics"))]
        let _unused = (metric, key, value);
    }
}
