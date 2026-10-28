//! The suite's settings from its own configuration sections, with their
//! defaults. Compiled in every build: without the `config` feature every
//! value is its default.

use std::{path::PathBuf, time::Duration};

/// The suite's settings read from the configuration.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Sections {
    pub(crate) guard: Guard,
    pub(crate) log: Log,
    pub(crate) crash: Crash,
    pub(crate) metrics: Metrics,
    pub(crate) kernel: Kernel,
    pub(crate) ids: Ids,
    pub(crate) audit: Audit,
    pub(crate) secrets: Secrets,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct Secrets {
    pub(crate) allow_plain: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct Guard {
    pub(crate) lock: Option<bool>,
    pub(crate) lock_dir: Option<PathBuf>,
    pub(crate) allow_network_lock: bool,
    pub(crate) umask: Option<u32>,
    pub(crate) open_files_target: Option<u64>,
    pub(crate) open_files_minimum: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogFormat {
    /// Human on a terminal, JSON otherwise.
    Auto,
    Human,
    Json,
    Logfmt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogRotation {
    Never,
    Hourly,
    Daily,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Log {
    pub(crate) level: LogLevel,
    pub(crate) filter: Option<String>,
    pub(crate) format: LogFormat,
    pub(crate) stderr: bool,
    pub(crate) file: bool,
    pub(crate) max_bytes: Option<u64>,
    pub(crate) rotation: LogRotation,
    pub(crate) max_files: usize,
    pub(crate) redact: Vec<String>,
    pub(crate) redact_common: bool,
    pub(crate) recent_lines: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Crash {
    pub(crate) enabled: Option<bool>,
    pub(crate) max_reports: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Metrics {
    pub(crate) enabled: bool,
    pub(crate) max_series: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Kernel {
    pub(crate) shutdown_deadline: Option<Duration>,
    pub(crate) member_reserve: Option<Duration>,
    pub(crate) notify: bool,
    pub(crate) reload_signal: bool,
    pub(crate) dump_signal: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ids {
    pub(crate) worker: u16,
    pub(crate) epoch_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuditHash {
    Sha256,
    Blake3,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Audit {
    pub(crate) enabled: bool,
    pub(crate) file: Option<PathBuf>,
    pub(crate) hash: AuditHash,
    /// `sync = "every"`: on disk before `record` returns; `"flush"`: synced
    /// every second and at shutdown.
    pub(crate) sync_every: bool,
    pub(crate) repair_tail: bool,
}

impl Default for Sections {
    fn default() -> Self {
        Sections {
            guard: Guard::default(),
            log: Log {
                level: LogLevel::Info,
                filter: None,
                format: LogFormat::Auto,
                stderr: true,
                file: true,
                max_bytes: None,
                rotation: LogRotation::Daily,
                max_files: 14,
                redact: Vec::new(),
                redact_common: true,
                recent_lines: 64,
            },
            crash: Crash {
                enabled: None,
                max_reports: 8,
            },
            metrics: Metrics {
                enabled: true,
                max_series: 10_000,
            },
            kernel: Kernel {
                shutdown_deadline: None,
                member_reserve: None,
                notify: true,
                reload_signal: true,
                dump_signal: true,
            },
            ids: Ids {
                worker: 0,
                epoch_ms: None,
            },
            audit: Audit {
                enabled: true,
                file: None,
                hash: AuditHash::Sha256,
                sync_every: true,
                repair_tail: false,
            },
            secrets: Secrets::default(),
        }
    }
}
