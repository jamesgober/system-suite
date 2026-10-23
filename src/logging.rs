//! The suite's own log lines.
//!
//! The logger exists only from boot step 10, but threads started earlier
//! (the signal intake) and the crash hook write to it. They share a slot
//! that the logging step fills once. Without the `log` feature every call is
//! a no-op.

#[cfg(feature = "log")]
use std::sync::{Arc, OnceLock};

/// One value of a structured field.
// Without the `log` feature the values are built and dropped unread.
#[cfg_attr(not(feature = "log"), allow(dead_code))]
#[derive(Clone, Copy, Debug)]
pub(crate) enum Val<'a> {
    Str(&'a str),
    U64(u64),
}

/// A shared, set-once handle to the system's logger.
#[derive(Clone, Default)]
pub(crate) struct LogSlot {
    #[cfg(feature = "log")]
    logger: Arc<OnceLock<log_io::Logger>>,
}

impl LogSlot {
    /// Fills the slot. Returns `false` if it was already filled.
    #[cfg(feature = "log")]
    pub(crate) fn set(&self, logger: log_io::Logger) -> bool {
        self.logger.set(logger).is_ok()
    }

    /// The logger, once the logging step ran.
    #[cfg(all(feature = "log", any(test, feature = "kernel")))]
    pub(crate) fn get(&self) -> Option<&log_io::Logger> {
        self.logger.get()
    }

    pub(crate) fn info(&self, message: &str, fields: &[(&str, Val<'_>)]) {
        self.write(Level::Info, message, fields);
    }

    #[cfg_attr(not(any(feature = "log", feature = "kernel")), allow(dead_code))]
    pub(crate) fn warn(&self, message: &str, fields: &[(&str, Val<'_>)]) {
        self.write(Level::Warn, message, fields);
    }

    pub(crate) fn error(&self, message: &str, fields: &[(&str, Val<'_>)]) {
        self.write(Level::Error, message, fields);
    }

    #[cfg(feature = "log")]
    fn write(&self, level: Level, message: &str, fields: &[(&str, Val<'_>)]) {
        let Some(logger) = self.logger.get() else {
            return;
        };
        let level = match level {
            Level::Info => log_io::Level::Info,
            Level::Warn => log_io::Level::Warn,
            Level::Error => log_io::Level::Error,
        };
        let converted: Vec<log_io::Field<'_>> = fields
            .iter()
            .map(|(key, value)| match value {
                Val::Str(text) => log_io::Field::new(key, *text),
                Val::U64(number) => log_io::Field::new(key, *number),
            })
            .collect();
        logger.log(level, message, &converted);
    }

    #[cfg(not(feature = "log"))]
    fn write(&self, _level: Level, _message: &str, _fields: &[(&str, Val<'_>)]) {}
}

#[cfg_attr(not(feature = "log"), allow(dead_code))]
#[derive(Clone, Copy)]
enum Level {
    Info,
    Warn,
    Error,
}

#[cfg(all(test, feature = "log"))]
mod tests {
    use super::*;

    #[test]
    fn test_log_slot_is_set_once() {
        let slot = LogSlot::default();
        assert!(slot.get().is_none());
        slot.info("before the logger: dropped", &[]);
        let logger = log_io::Logger::builder()
            .with_sink(log_io::sink::NullSink::new())
            .build();
        assert!(slot.set(logger.clone()));
        assert!(!slot.set(logger));
        assert!(slot.get().is_some());
        slot.warn("written", &[("n", Val::U64(1)), ("s", Val::Str("x"))]);
    }
}
