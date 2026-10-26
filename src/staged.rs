//! `Boot`: the staged start for frameworks (boot steps 16 and 17 under the
//! caller's control).

use std::{
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
};

use crate::{
    Error, ErrorKind, Foundation, Step, System,
    boot::Parts,
    builder::{OnReady, validate_name},
    kernel::{Kernel, Slot},
    lifecycle::{Link, Mailbox, Subsystem},
    logging::Val,
};

/// A system whose foundation and kernel are running (boot steps 1 to 15)
/// and whose subsystems the caller starts.
///
/// A framework's own boot phases happen between the foundation and
/// readiness, and each phase builds things that need the previous phase's
/// results. [`start`](Self::start) starts a subsystem at once and appends it
/// to the stop order; [`ready`](Self::ready) starts the subsystems given to
/// [`Builder::register`](crate::Builder::register), runs the `on_ready`
/// callbacks, announces readiness, and returns the [`System`].
///
/// Dropping a `Boot` stops everything it started, in reverse, as a boot
/// failure does. A stop requested while a `Boot` exists makes the next
/// `start` or `ready` fail with [`Interrupted`](ErrorKind::Interrupted).
///
/// # Examples
///
/// ```
/// use system_suite::{
///     Member, System,
///     lifecycle::{Deadline, Failure, Link, Subsystem},
/// };
///
/// struct Listener;
///
/// impl Subsystem for Listener {
///     fn name(&self) -> &str {
///         "listener"
///     }
///     fn start(&mut self, _link: &Link) -> Result<(), Failure> {
///         Ok(())
///     }
///     fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> {
///         Ok(())
///     }
/// }
///
/// let builder = System::builder("boot-doc")
///     .process_wide(false)
///     .disable(Member::Log);
/// # #[cfg(feature = "sys")]
/// # let builder = builder.single_instance(false);
/// let mut boot = builder.boot()?;
/// boot.start(Listener)?;
/// let system = boot.ready()?;
/// let code = system.shutdown();
/// # let _ = code;
/// # Ok::<(), system_suite::Error>(())
/// ```
#[must_use = "dropping a Boot stops everything it started"]
pub struct Boot {
    parts: Option<Parts>,
    foundation: Foundation,
    registered: Vec<Box<dyn Subsystem>>,
    on_ready: Vec<OnReady>,
}

impl Boot {
    pub(crate) fn new(
        parts: Parts,
        foundation: Foundation,
        registered: Vec<Box<dyn Subsystem>>,
        on_ready: Vec<OnReady>,
    ) -> Boot {
        Boot {
            parts: Some(parts),
            foundation,
            registered,
            on_ready,
        }
    }

    /// The typed handles of every enabled member.
    #[must_use]
    pub fn foundation(&self) -> &Foundation {
        &self.foundation
    }

    /// The kernel's control handle.
    #[must_use]
    pub fn kernel(&self) -> &Kernel {
        self.foundation.kernel()
    }

    /// Starts `subsystem` now, on the calling thread, and appends it to the
    /// stop order.
    ///
    /// # Errors
    ///
    /// [`Setup`](ErrorKind::Setup) for an invalid or duplicate name;
    /// [`Subsystem`](ErrorKind::Subsystem) when its `start` fails or
    /// panics; [`Interrupted`](ErrorKind::Interrupted) once a stop was
    /// requested. On error the subsystem is not added and the `Boot` is
    /// still usable.
    pub fn start(&mut self, subsystem: impl Subsystem) -> Result<(), Error> {
        self.start_boxed(Box::new(subsystem))
    }

    fn start_boxed(&mut self, mut subsystem: Box<dyn Subsystem>) -> Result<(), Error> {
        let name = subsystem.name().to_owned();
        let kernel = self.foundation.kernel().clone();
        let Some(parts) = &self.parts else {
            return Err(internal());
        };
        parts.check_interrupted(Step::Subsystems)?;
        validate_name(&name, "subsystem").map_err(|e| e.at(Step::Subsystems))?;
        if kernel.slots().iter().any(|slot| slot.name == name) {
            return Err(Error::new(
                ErrorKind::Setup,
                format!("two subsystems are named {name}"),
                "give each subsystem its own name",
            )
            .at(Step::Subsystems));
        }
        let mailbox = Mailbox::new();
        let link = Link::new(&name, self.foundation.clone(), mailbox.clone());
        let began = std::time::Instant::now();
        let started = catch_unwind(AssertUnwindSafe(|| subsystem.start(&link)));
        let failure = match started {
            Ok(Ok(())) => None,
            Ok(Err(failure)) => Some(
                Error::new(
                    ErrorKind::Subsystem,
                    format!("subsystem {name} failed to start: {failure}"),
                    "fix the cause in the message; the subsystem was not started",
                )
                .caused_by(failure.into_error()),
            ),
            Err(_) => Some(Error::new(
                ErrorKind::Subsystem,
                format!("subsystem {name} panicked while starting"),
                "fix the subsystem; it was not started",
            )),
        };
        if let Some(error) = failure {
            return Err(error.in_subsystem(&name).at(Step::Subsystems));
        }
        let health = subsystem.health();
        kernel.add_slot(Arc::new(Slot {
            name: name.clone(),
            subsystem: Mutex::new(subsystem),
            last: Mutex::new(health),
            mailbox,
        }));
        parts.log.info(
            "subsystem started",
            &[
                ("subsystem", Val::Str(&name)),
                ("millis", Val::U64(crate::boot::millis(began.elapsed()))),
            ],
        );
        Ok(())
    }

    /// Starts the registered subsystems in ascending
    /// [`Position`](crate::lifecycle::Position) (ties in registration
    /// order), runs the `on_ready` callbacks, announces readiness, and
    /// returns the running system.
    ///
    /// # Errors
    ///
    /// As for [`start`](Self::start), and
    /// [`Subsystem`](ErrorKind::Subsystem) for a failing `on_ready`
    /// callback. On error everything started is stopped in reverse.
    pub fn ready(mut self) -> Result<System, Error> {
        match self.finish_boot() {
            Ok(()) => {
                let parts = self.parts.take().ok_or_else(internal)?;
                Ok(System::ready(parts, self.foundation.clone()))
            }
            Err(error) => match self.parts.take() {
                Some(mut parts) => Err(parts.fail(error)),
                None => Err(error),
            },
        }
    }

    fn finish_boot(&mut self) -> Result<(), Error> {
        let mut registered = std::mem::take(&mut self.registered);
        // A stable sort keeps registration order among equal positions.
        registered.sort_by_key(|subsystem| subsystem.position());
        for subsystem in registered {
            self.start_boxed(subsystem)?;
        }
        let parts = self.parts.as_ref().ok_or_else(internal)?;
        parts.check_interrupted(Step::Ready)?;
        for callback in std::mem::take(&mut self.on_ready) {
            let outcome = catch_unwind(AssertUnwindSafe(|| callback(&self.foundation)));
            let error = match outcome {
                Ok(Ok(())) => continue,
                Ok(Err(failure)) => Error::new(
                    ErrorKind::Subsystem,
                    format!("an on_ready callback failed: {failure}"),
                    "fix the cause in the message",
                )
                .caused_by(failure.into_error()),
                Err(_) => Error::new(
                    ErrorKind::Subsystem,
                    "an on_ready callback panicked",
                    "fix the callback",
                ),
            };
            return Err(error.at(Step::Ready));
        }
        parts.check_interrupted(Step::Ready)
    }
}

impl Drop for Boot {
    fn drop(&mut self) {
        if let Some(mut parts) = self.parts.take() {
            let _code = parts.teardown(crate::exit::OK, "the Boot was dropped");
        }
    }
}

impl fmt::Debug for Boot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Boot")
            .field("name", &self.foundation.name())
            .finish_non_exhaustive()
    }
}

fn internal() -> Error {
    Error::new(
        ErrorKind::Kernel,
        "internal: the Boot has already finished",
        "report this as a bug in system-suite",
    )
}
