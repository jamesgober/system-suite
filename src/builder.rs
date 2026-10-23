//! `Builder`: the boot's settings.

#[cfg(any(feature = "paths", feature = "env"))]
use std::path::PathBuf;
#[cfg(feature = "time")]
use std::sync::Arc;
use std::time::Duration;

use crate::{Error, ErrorKind, Member, System};

/// The default shutdown deadline: below Kubernetes' default 30 s grace
/// period and systemd's 90 s, so the suite reports what was late before it
/// is killed (D-024).
pub(crate) const DEFAULT_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(25);

/// The longest shutdown deadline accepted.
const MAX_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(3600);

/// Settings for the boot.
///
/// Every setting has a secure default. Invalid settings are reported by
/// [`start`](Builder::start) as an error of kind
/// [`Setup`](ErrorKind::Setup), never by the setter, so a chain of setters
/// never needs `?`. [`start!`](crate::start) covers the common case;
/// `Builder` is for services that configure their boot.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use system_suite::{Member, System};
///
/// # #[cfg(feature = "sys")] {
/// let system = System::builder("builder-doc")
///     .process_wide(false)          // touch no process-wide state (tests, embedding)
///     .single_instance(false)       // several copies may run
///     .disable(Member::Log)         // no log sinks
///     .shutdown_deadline(Duration::from_secs(5))
///     .start()?;
/// assert_eq!(system.name(), "builder-doc");
/// let code = system.shutdown();
/// # let _ = code;
/// # }
/// # Ok::<(), system_suite::Error>(())
/// ```
#[must_use = "a Builder does nothing until start()"]
pub struct Builder {
    pub(crate) settings: Settings,
}

/// The settings a `Builder` collects.
pub(crate) struct Settings {
    pub(crate) name: String,
    #[cfg(feature = "sys")]
    pub(crate) build: Option<&'static sys_lib::BuildInfo>,
    #[cfg(feature = "sys")]
    pub(crate) single_instance: bool,
    #[cfg(feature = "paths")]
    pub(crate) scope: Option<path_lib::Scope>,
    #[cfg(feature = "paths")]
    pub(crate) portable: Option<PathBuf>,
    #[cfg(feature = "env")]
    pub(crate) env_file: Option<PathBuf>,
    #[cfg(feature = "time")]
    pub(crate) clock: Option<Arc<dyn clock_lib::Clock>>,
    #[cfg(feature = "kernel")]
    pub(crate) crash_capture: bool,
    pub(crate) disabled: Vec<Member>,
    pub(crate) process_wide: bool,
    pub(crate) shutdown_deadline: Duration,
}

impl Settings {
    #[cfg(any(
        feature = "env",
        feature = "log",
        feature = "paths",
        feature = "signals"
    ))]
    pub(crate) fn is_disabled(&self, member: Member) -> bool {
        self.disabled.contains(&member)
    }
}

impl Builder {
    /// A builder for the application `name`: 1 to 64 ASCII letters,
    /// digits, `-`, `_`, `.`, not starting with `.`. The name names the
    /// directories, the lock, the environment prefix (`SITE_` for `site`),
    /// and the log target.
    pub fn new(name: &str) -> Builder {
        Builder {
            settings: Settings {
                name: name.to_owned(),
                #[cfg(feature = "sys")]
                build: None,
                #[cfg(feature = "sys")]
                single_instance: true,
                #[cfg(feature = "paths")]
                scope: None,
                #[cfg(feature = "paths")]
                portable: None,
                #[cfg(feature = "env")]
                env_file: None,
                #[cfg(feature = "time")]
                clock: None,
                #[cfg(feature = "kernel")]
                crash_capture: true,
                disabled: Vec::new(),
                process_wide: true,
                shutdown_deadline: DEFAULT_SHUTDOWN_DEADLINE,
            },
        }
    }

    /// The application's build info, from
    /// [`build_info!`](crate::build_info) expanded in the application crate.
    /// [`start!`](crate::start) sets it; without it,
    /// [`Foundation::build`](crate::Foundation::build) is `None` and the
    /// first log lines say so.
    #[cfg(feature = "sys")]
    pub fn build_info(mut self, build: &'static sys_lib::BuildInfo) -> Self {
        self.settings.build = Some(build);
        self
    }

    /// Where the application's directories live: `User` (the default),
    /// `Global` for system services. Without this call, the
    /// `<NAME>_PATH_SCOPE` variable (`user`, `global`, `auto`) decides.
    /// `Scope::Portable` needs [`portable`](Self::portable) instead.
    #[cfg(feature = "paths")]
    pub fn scope(mut self, scope: path_lib::Scope) -> Self {
        self.settings.scope = Some(scope);
        self
    }

    /// Every directory under `root` (`root/data`, `root/config`, ...).
    #[cfg(feature = "paths")]
    pub fn portable(mut self, root: impl Into<PathBuf>) -> Self {
        self.settings.portable = Some(root.into());
        self
    }

    /// The `.env` file to read instead of `<config dir>/.env`. An explicit
    /// file that does not exist is an error of kind
    /// [`Environment`](ErrorKind::Environment).
    #[cfg(feature = "env")]
    pub fn env_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.settings.env_file = Some(path.into());
        self
    }

    /// The clock [`Foundation::clock`](crate::Foundation::clock) returns: a
    /// `ManualClock` in tests. Kernel deadlines always use the OS monotonic
    /// clock (D-029).
    #[cfg(feature = "time")]
    pub fn clock(mut self, clock: impl clock_lib::Clock + 'static) -> Self {
        self.settings.clock = Some(Arc::new(clock));
        self
    }

    /// Turns a compiled-in member off for this run. `Env`, `Log`, and
    /// `Signals` can be disabled in this version (more from `0.4`);
    /// members others need at run time cannot, and asking is a
    /// [`Setup`](ErrorKind::Setup) error.
    pub fn disable(mut self, member: Member) -> Self {
        if !self.settings.disabled.contains(&member) {
            self.settings.disabled.push(member);
        }
        self
    }

    /// Whether the system may change process-wide state (default `true`):
    /// signal handlers, the panic hook, the umask, the open-file limit, the
    /// global logger, and systemd notifications. `false` for tests that
    /// start several systems in one process and for hosts that embed one.
    pub fn process_wide(mut self, on: bool) -> Self {
        self.settings.process_wide = on;
        self
    }

    /// Whether to install the crash hook (default `true`, D-016). It is
    /// never installed with `process_wide(false)`.
    #[cfg(feature = "kernel")]
    pub fn crash_capture(mut self, on: bool) -> Self {
        self.settings.crash_capture = on;
        self
    }

    /// Whether to take the single-instance lock (default `true`). `false`
    /// when a supervisor holds the lock or several copies are meant to run.
    #[cfg(feature = "sys")]
    pub fn single_instance(mut self, on: bool) -> Self {
        self.settings.single_instance = on;
        self
    }

    /// The whole shutdown's deadline (default 25 s). Zero, or more than an
    /// hour, is a [`Setup`](ErrorKind::Setup) error.
    pub fn shutdown_deadline(mut self, deadline: Duration) -> Self {
        self.settings.shutdown_deadline = deadline;
        self
    }

    /// Runs every boot step and returns the system when all succeeded. On
    /// failure nothing started is left running, and the error names the
    /// step, the member, and what to do.
    ///
    /// # Errors
    ///
    /// [`Setup`](ErrorKind::Setup) for an invalid setting; otherwise the
    /// kind of the step that failed (see [`ErrorKind`]).
    pub fn start(self) -> Result<System, Error> {
        crate::boot::start(self.settings)
    }
}

/// Checks the settings before anything runs.
pub(crate) fn validate(settings: &Settings) -> Result<(), Error> {
    validate_name(&settings.name)?;
    if let Some(member) = settings.disabled.iter().find(|m| !m.can_disable()) {
        return Err(setup(
            format!("the {member} member cannot be disabled at run time"),
            "remove the disable() call, or turn its feature off at compile time",
        ));
    }
    if settings.shutdown_deadline.is_zero() || settings.shutdown_deadline > MAX_SHUTDOWN_DEADLINE {
        return Err(setup(
            "the shutdown deadline must be more than zero and at most one hour",
            "pass a deadline between 1 ms and 1 h to shutdown_deadline()",
        ));
    }
    #[cfg(feature = "paths")]
    if settings.scope == Some(path_lib::Scope::Portable) && settings.portable.is_none() {
        return Err(setup(
            "Scope::Portable needs a root directory",
            "call portable(root) instead of scope(Scope::Portable)",
        ));
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), Error> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(setup(
            format!("the application name {name:?} is not valid"),
            "use 1 to 64 ASCII letters, digits, '-', '_', or '.', not starting with '.'",
        ))
    }
}

fn setup(detail: impl Into<String>, action: &'static str) -> Error {
    Error::new(ErrorKind::Setup, detail, action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_name_accepts_the_documented_set() {
        for name in ["site", "hive-db", "a_b.c", "x", &"a".repeat(64)] {
            assert!(validate_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn test_validate_name_refuses_bad_names() {
        for name in [
            "",
            ".hidden",
            "has space",
            "slash/name",
            "é",
            &"a".repeat(65),
        ] {
            let error = validate_name(name).err();
            assert_eq!(error.map(|e| e.kind()), Some(ErrorKind::Setup), "{name}");
        }
    }

    #[test]
    fn test_validate_refuses_undisableable_member() {
        let builder = Builder::new("app").disable(Member::Kernel);
        let error = validate(&builder.settings).err();
        assert_eq!(error.map(|e| e.kind()), Some(ErrorKind::Setup));
    }

    #[test]
    fn test_validate_refuses_bad_deadlines() {
        for deadline in [Duration::ZERO, Duration::from_secs(3601)] {
            let builder = Builder::new("app").shutdown_deadline(deadline);
            assert!(validate(&builder.settings).is_err(), "{deadline:?}");
        }
        let builder = Builder::new("app").shutdown_deadline(Duration::from_secs(3600));
        assert!(validate(&builder.settings).is_ok());
    }

    #[cfg(feature = "paths")]
    #[test]
    fn test_validate_refuses_portable_scope_without_root() {
        let builder = Builder::new("app").scope(path_lib::Scope::Portable);
        assert!(validate(&builder.settings).is_err());
    }

    #[test]
    fn test_disable_is_idempotent() {
        let builder = Builder::new("app")
            .disable(Member::Log)
            .disable(Member::Log);
        assert_eq!(builder.settings.disabled, vec![Member::Log]);
    }
}
