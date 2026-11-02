//! The suite's own sections: `[guard]`, `[log]`, `[crash]`, `[metrics]`,
//! `[kernel]`, `[ids]`.
//!
//! Every section is optional and every key has a secure default. An unknown
//! key in one of these sections is refused, so a typo cannot silently leave
//! a setting at its default. Other sections belong to the application.

use super::{Config, FromConfig};
pub(crate) use crate::sections::{AuditHash, LogFormat, LogLevel, LogRotation, Sections};
use crate::{Error, ErrorKind};

const KNOWN: &[(&str, &[&str])] = &[
    (
        "guard",
        &[
            "lock",
            "lock_dir",
            "allow_network_lock",
            "umask",
            "open_files_target",
            "open_files_minimum",
        ],
    ),
    (
        "log",
        &[
            "level",
            "filter",
            "format",
            "stderr",
            "file",
            "max_bytes",
            "rotation",
            "max_files",
            "redact",
            "redact_common",
            "recent_lines",
        ],
    ),
    ("crash", &["enabled", "max_reports"]),
    ("metrics", &["enabled", "max_series"]),
    (
        "kernel",
        &[
            "shutdown_deadline",
            "member_reserve",
            "notify",
            "reload_signal",
            "dump_signal",
        ],
    ),
    ("ids", &["worker", "epoch"]),
    ("audit", &["enabled", "file", "hash", "sync", "repair_tail"]),
    ("secrets", &["allow_plain"]),
];

/// Reads and validates the suite's sections.
pub(crate) fn read(config: &Config) -> Result<Sections, Error> {
    for (section, keys) in KNOWN {
        if let Some(value) = config.value(section) {
            let config_lib::Value::Table(table) = value else {
                return Err(invalid(
                    format!("`{section}` must be a section (`[{section}]`), not a value"),
                    format!("write it as a [{section}] section"),
                ));
            };
            if let Some(unknown) = table.keys().find(|k| !keys.contains(&k.as_str())) {
                return Err(invalid(
                    format!("unknown key `{section}.{unknown}` in the suite's [{section}] section"),
                    format!(
                        "check the spelling; the keys of [{section}] are: {}",
                        keys.join(", ")
                    ),
                ));
            }
        }
    }
    let mut s = Sections::default();

    s.guard.lock = config.get("guard.lock")?;
    s.guard.lock_dir = config.get("guard.lock_dir")?;
    s.guard.allow_network_lock = config.get_or("guard.allow_network_lock", false)?;
    s.guard.umask = match config.get::<String>("guard.umask")? {
        Some(text) => Some(parse_umask(&text)?),
        None => None,
    };
    s.guard.open_files_target = config.get("guard.open_files_target")?;
    s.guard.open_files_minimum = config.get("guard.open_files_minimum")?;

    if let Some(level) = config.get::<String>("log.level")? {
        s.log.level = match level.as_str() {
            "trace" => LogLevel::Trace,
            "debug" => LogLevel::Debug,
            "info" => LogLevel::Info,
            "warn" => LogLevel::Warn,
            "error" => LogLevel::Error,
            _ => return Err(choice("log.level", "trace, debug, info, warn, or error")),
        };
    }
    s.log.filter = config.get("log.filter")?;
    if let Some(format) = config.get::<String>("log.format")? {
        s.log.format = match format.as_str() {
            "auto" => LogFormat::Auto,
            "human" => LogFormat::Human,
            "json" => LogFormat::Json,
            "logfmt" => LogFormat::Logfmt,
            _ => return Err(choice("log.format", "auto, human, json, or logfmt")),
        };
    }
    s.log.stderr = config.get_or("log.stderr", s.log.stderr)?;
    s.log.file = config.get_or("log.file", s.log.file)?;
    s.log.max_bytes = config.get("log.max_bytes")?;
    if let Some(rotation) = config.get::<String>("log.rotation")? {
        s.log.rotation = match rotation.as_str() {
            "never" => LogRotation::Never,
            "hourly" => LogRotation::Hourly,
            "daily" => LogRotation::Daily,
            _ => return Err(choice("log.rotation", "never, hourly, or daily")),
        };
    }
    s.log.max_files = bounded(config, "log.max_files", s.log.max_files, 0, 1_000)?;
    s.log.redact = config.get_or("log.redact", Vec::new())?;
    s.log.redact_common = config.get_or("log.redact_common", s.log.redact_common)?;
    s.log.recent_lines = bounded(config, "log.recent_lines", s.log.recent_lines, 0, 10_000)?;

    s.crash.enabled = config.get("crash.enabled")?;
    s.crash.max_reports = bounded(config, "crash.max_reports", s.crash.max_reports, 0, 1_000)?;

    s.metrics.enabled = config.get_or("metrics.enabled", s.metrics.enabled)?;
    s.metrics.max_series = bounded(
        config,
        "metrics.max_series",
        s.metrics.max_series,
        1,
        10_000_000,
    )?;

    s.kernel.shutdown_deadline = config.get("kernel.shutdown_deadline")?;
    s.kernel.member_reserve = config.get("kernel.member_reserve")?;
    // Checked here so a reload is held to the same rules as the boot.
    let deadline = s
        .kernel
        .shutdown_deadline
        .unwrap_or(crate::builder::DEFAULT_SHUTDOWN_DEADLINE);
    if deadline.is_zero() || deadline > crate::builder::MAX_SHUTDOWN_DEADLINE {
        return Err(invalid(
            "`kernel.shutdown_deadline` must be more than zero and at most one hour",
            "set `kernel.shutdown_deadline` to a duration from \"1ms\" to \"1h\"",
        ));
    }
    if let Some(reserve) = s.kernel.member_reserve
        && reserve >= deadline
    {
        return Err(invalid(
            "`kernel.member_reserve` must be shorter than the shutdown deadline",
            "set `kernel.member_reserve` below `kernel.shutdown_deadline`",
        ));
    }
    if let Some(notify) = config.get::<String>("kernel.notify")? {
        s.kernel.notify = match notify.as_str() {
            "auto" => true,
            "off" => false,
            _ => return Err(choice("kernel.notify", "auto or off")),
        };
    }
    s.kernel.reload_signal = config.get_or("kernel.reload_signal", s.kernel.reload_signal)?;
    s.kernel.dump_signal = config.get_or("kernel.dump_signal", s.kernel.dump_signal)?;

    s.ids.worker = bounded(config, "ids.worker", s.ids.worker, 0, 1_023)?;
    s.ids.epoch_ms = config.get("ids.epoch")?;
    if let Some(epoch) = s.ids.epoch_ms {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        if epoch > now {
            return Err(invalid(
                "`ids.epoch` is in the future: Snowflake ids would stall",
                "set `ids.epoch` to a past time in milliseconds since 1970",
            ));
        }
    }

    s.audit.enabled = config.get_or("audit.enabled", s.audit.enabled)?;
    s.audit.file = config.get("audit.file")?;
    if let Some(hash) = config.get::<String>("audit.hash")? {
        s.audit.hash = match hash.as_str() {
            "sha256" => AuditHash::Sha256,
            "blake3" if cfg!(feature = "audit-blake3") => AuditHash::Blake3,
            "blake3" => {
                return Err(invalid(
                    "`audit.hash = \"blake3\"` needs the suite's `audit-blake3` feature",
                    "enable the `audit-blake3` feature, or use \"sha256\"",
                ));
            }
            _ => return Err(choice("audit.hash", "sha256 or blake3")),
        };
    }
    if let Some(sync) = config.get::<String>("audit.sync")? {
        s.audit.sync_every = match sync.as_str() {
            "every" => true,
            "flush" => false,
            _ => return Err(choice("audit.sync", "every or flush")),
        };
    }
    s.audit.repair_tail = config.get_or("audit.repair_tail", s.audit.repair_tail)?;
    s.secrets.allow_plain = config.get_or("secrets.allow_plain", s.secrets.allow_plain)?;
    Ok(s)
}

fn bounded<T>(config: &Config, key: &str, default: T, min: T, max: T) -> Result<T, Error>
where
    T: FromConfig + PartialOrd + Copy + std::fmt::Display,
{
    let value = config.get_or(key, default)?;
    if value < min || value > max {
        return Err(invalid(
            format!("`{key}` must be from {min} to {max}"),
            format!("set `{key}` to a value from {min} to {max}"),
        ));
    }
    Ok(value)
}

/// `"0027"` or `"027"`: an octal mode mask of at most `0777`.
fn parse_umask(text: &str) -> Result<u32, Error> {
    match u32::from_str_radix(text.trim_start_matches("0o"), 8) {
        Ok(mask) if mask <= 0o777 => Ok(mask),
        _ => Err(invalid(
            "`guard.umask` must be an octal mask such as \"0027\"",
            "set `guard.umask` to an octal string from \"0000\" to \"0777\"",
        )),
    }
}

fn choice(key: &str, choices: &str) -> Error {
    invalid(
        format!("`{key}` must be one of: {choices}"),
        format!("set `{key}` to one of: {choices}"),
    )
}

fn invalid(detail: impl Into<String>, action: impl Into<std::borrow::Cow<'static, str>>) -> Error {
    Error::new(ErrorKind::ConfigInvalid, detail, action)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn sections(text: &str) -> Result<Sections, Error> {
        read(&Config::from_text(text)?)
    }

    #[test]
    fn test_empty_configuration_takes_every_default() {
        assert_eq!(sections("").ok(), Some(Sections::default()));
    }

    #[test]
    fn test_values_are_read_and_checked() {
        let s = sections(
            "[log]\nlevel = \"debug\"\nformat = \"json\"\nrecent_lines = 128\n\
             [kernel]\nshutdown_deadline = \"40s\"\nnotify = \"off\"\n\
             [guard]\numask = \"0077\"\nlock = false\n[ids]\nworker = 12\n",
        );
        let s = s.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.log.level, LogLevel::Debug);
        assert_eq!(s.log.format, LogFormat::Json);
        assert_eq!(s.log.recent_lines, 128);
        assert_eq!(s.kernel.shutdown_deadline, Some(Duration::from_secs(40)));
        assert!(!s.kernel.notify);
        assert_eq!(s.guard.umask, Some(0o077));
        assert_eq!(s.guard.lock, Some(false));
        assert_eq!(s.ids.worker, 12);
    }

    #[test]
    fn test_unknown_keys_and_bad_values_are_refused() {
        for text in [
            "[log]\nlevle = \"debug\"\n",
            "[log]\nlevel = \"loud\"\n",
            "[ids]\nworker = 2000\n",
            "[guard]\numask = \"999\"\n",
            "[kernel]\nnotify = \"maybe\"\n",
            "[kernel]\nshutdown_deadline = \"0s\"\n",
            "[kernel]\nshutdown_deadline = \"2h\"\n",
            "[kernel]\nshutdown_deadline = \"5s\"\nmember_reserve = \"5s\"\n",
            "[ids]\nepoch = 99999999999999\n",
            "log = 3\n",
        ] {
            let error = sections(text).err();
            assert_eq!(
                error.map(|e| e.kind()),
                Some(ErrorKind::ConfigInvalid),
                "{text}"
            );
        }
    }

    #[test]
    fn test_application_sections_are_left_alone() {
        assert!(sections("[http]\nport = 8080\nanything = \"goes\"\n").is_ok());
    }
}
