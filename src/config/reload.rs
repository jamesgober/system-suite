//! The two-phase reload (D-019): read and validate, let every subsystem
//! prepare, then publish and commit, or change nothing (threat T7).

use std::sync::{Mutex, PoisonError};

use super::{
    Config,
    load::{self, Source},
    sections::{self, Sections},
};
use crate::{
    Error, ErrorKind, Snapshot,
    kernel::Kernel,
    lifecycle::Commit,
    logging::{LogSlot, Val},
};

/// Everything a reload needs, shared by `System::reload` and the reload
/// signal. Reloads are serialised.
pub(crate) struct Reloader {
    source: Source,
    snapshot: Snapshot<Config>,
    sections: Mutex<Sections>,
    running: Mutex<()>,
    kernel: Kernel,
    log: LogSlot,
}

impl Reloader {
    pub(crate) fn new(
        source: Source,
        snapshot: Snapshot<Config>,
        sections: Sections,
        kernel: Kernel,
        log: LogSlot,
    ) -> Reloader {
        Reloader {
            source,
            snapshot,
            sections: Mutex::new(sections),
            running: Mutex::new(()),
            kernel,
            log,
        }
    }

    /// The suite's sections as currently in force.
    pub(crate) fn sections(&self) -> Sections {
        self.sections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Reloads, logs the outcome, and counts it.
    pub(crate) fn reload(&self) -> Result<u64, Error> {
        let result = self.try_reload();
        match &result {
            Ok(version) => {
                self.log
                    .info("configuration reloaded", &[("version", Val::U64(*version))]);
            }
            Err(error) => {
                self.log.warn(
                    "configuration reload refused; the old configuration stays",
                    &[("error", Val::Str(&error.to_string()))],
                );
            }
        }
        if let Some(metrics) = self.kernel.metrics() {
            metrics.reload(result.is_ok());
        }
        result
    }

    fn try_reload(&self) -> Result<u64, Error> {
        let _one_at_a_time = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if self.kernel.stop_reason().is_some() {
            return Err(Error::new(
                ErrorKind::Stopping,
                "a stop was requested; the configuration is not reloaded",
                "nothing: the process is going away",
            ));
        }
        let version = self.snapshot.version().saturating_add(1);
        let config = load::load(&self.source, version)?;
        let new_sections = sections::read(&config)?;

        // Phase one: every subsystem prepares; any refusal changes nothing.
        let mut commits: Vec<Commit> = Vec::new();
        for slot in self.kernel.slots() {
            let mut subsystem = slot
                .subsystem
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let prepared = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                subsystem.reload(&config)
            }));
            match prepared {
                Ok(Ok(commit)) => commits.push(commit),
                Ok(Err(failure)) => {
                    return Err(Error::new(
                        ErrorKind::ReloadRefused,
                        format!("subsystem {} refused the new configuration: {failure}", slot.name),
                        "fix the configuration as the subsystem's message says; nothing was changed",
                    )
                    .in_subsystem(&slot.name)
                    .caused_by(failure.into_error()));
                }
                Err(_) => {
                    return Err(Error::new(
                        ErrorKind::ReloadRefused,
                        format!("subsystem {} panicked while preparing a reload", slot.name),
                        "fix the subsystem; nothing was changed",
                    )
                    .in_subsystem(&slot.name));
                }
            }
        }

        // Phase two: publish, then commit. Nothing here can fail.
        let old_sections = {
            let mut current = self.sections.lock().unwrap_or_else(PoisonError::into_inner);
            std::mem::replace(&mut *current, new_sections.clone())
        };
        let published = self.snapshot.publish(config);
        for commit in commits {
            commit.apply();
        }
        self.note_restart_only(&old_sections, &new_sections);
        Ok(published)
    }

    /// Logs the sections whose changes take effect at the next start.
    fn note_restart_only(&self, old: &Sections, new: &Sections) {
        let changed = [
            ("guard", old.guard != new.guard),
            ("log", old.log != new.log),
            ("crash", old.crash != new.crash),
            ("metrics", old.metrics != new.metrics),
            ("ids", old.ids != new.ids),
            (
                "kernel signals",
                old.kernel.notify != new.kernel.notify
                    || old.kernel.reload_signal != new.kernel.reload_signal
                    || old.kernel.dump_signal != new.kernel.dump_signal,
            ),
        ];
        for (section, differs) in changed {
            if differs {
                self.log.info(
                    "configuration change takes effect at restart",
                    &[("section", Val::Str(section))],
                );
            }
        }
    }
}
