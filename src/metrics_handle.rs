//! The system's metrics registry: the process-wide one, or the system's own.

use std::{ops::Deref, sync::Arc};

use metrics_lib::MetricsCore;

/// metrics-lib's registry as the system holds it. With `process_wide` it is
/// metrics-lib's global, so its macros (`counter!`, `gauge!`) record into
/// the same registry; otherwise the system owns one.
#[derive(Clone)]
pub(crate) enum MetricsRef {
    Global(&'static MetricsCore),
    Owned(Arc<MetricsCore>),
}

impl Deref for MetricsRef {
    type Target = MetricsCore;

    fn deref(&self) -> &MetricsCore {
        match self {
            MetricsRef::Global(core) => core,
            MetricsRef::Owned(core) => core,
        }
    }
}
