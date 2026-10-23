//! The crate's one error type.

use std::{
    borrow::Cow,
    error::Error as StdError,
    fmt,
    io::{self, Write as _},
    process::ExitCode,
};

use crate::{Member, Step, exit};

/// A `Result` whose error is [`Error`].
pub type Result<T, E = Error> = core::result::Result<T, E>;

type Source = Box<dyn StdError + Send + Sync + 'static>;

/// What went wrong: the kind, the boot step and member it happened in, and
/// what to do about it.
///
/// One error type for the whole crate. The member's own error (a
/// `path_lib::Error`, a `sys_lib::Error`, ...) is kept as
/// [`source`](std::error::Error::source). Neither `Display` nor `Debug`
/// ever contains a secret or a configuration value; they name keys and
/// files only.
///
/// # Examples
///
/// ```no_run
/// use system_suite::ErrorKind;
///
/// # #[cfg(feature = "kernel")]
/// fn main() -> std::process::ExitCode {
///     let system = match system_suite::start!() {
///         Ok(system) => system,
///         // A copy started by cron while the service runs: nothing to do.
///         Err(error) if error.kind() == ErrorKind::AlreadyRunning => {
///             return std::process::ExitCode::SUCCESS;
///         }
///         Err(error) => return error.report(),
///     };
///     system.run()
/// }
/// # #[cfg(not(feature = "kernel"))]
/// # fn main() {}
/// ```
pub struct Error(Box<Inner>);

struct Inner {
    kind: ErrorKind,
    step: Option<Step>,
    member: Option<Member>,
    subsystem: Option<String>,
    app: Option<String>,
    detail: String,
    action: Cow<'static, str>,
    source: Option<Source>,
}

impl Error {
    /// A new error of `kind`: `detail` says what failed, `action` what to do.
    pub(crate) fn new(
        kind: ErrorKind,
        detail: impl Into<String>,
        action: impl Into<Cow<'static, str>>,
    ) -> Self {
        Error(Box::new(Inner {
            kind,
            step: None,
            member: None,
            subsystem: None,
            app: None,
            detail: detail.into(),
            action: action.into(),
            source: None,
        }))
    }

    /// Records the boot step (and its member) unless one is already set.
    pub(crate) fn at(mut self, step: Step) -> Self {
        if self.0.step.is_none() {
            self.0.step = Some(step);
            if self.0.member.is_none() {
                self.0.member = step.member();
            }
        }
        self
    }

    /// Records the member the error came from.
    #[cfg(any(
        feature = "env",
        feature = "log",
        feature = "paths",
        feature = "signals",
        feature = "sys"
    ))]
    pub(crate) fn member_of(mut self, member: Member) -> Self {
        self.0.member = Some(member);
        self
    }

    /// Keeps the member's own error as the source.
    #[cfg(any(
        test,
        feature = "env",
        feature = "log",
        feature = "paths",
        feature = "signals",
        feature = "sys"
    ))]
    pub(crate) fn caused_by(mut self, source: impl StdError + Send + Sync + 'static) -> Self {
        self.0.source = Some(Box::new(source));
        self
    }

    /// Records the application name, used by [`report`](Self::report).
    pub(crate) fn for_app(mut self, name: &str) -> Self {
        if self.0.app.is_none() {
            self.0.app = Some(name.to_owned());
        }
        self
    }

    /// What kind of failure this is.
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        self.0.kind
    }

    /// The boot step that failed; `None` for errors after boot.
    #[must_use]
    pub fn step(&self) -> Option<Step> {
        self.0.step
    }

    /// The member the failure came from, when it came from one.
    #[must_use]
    pub fn member(&self) -> Option<Member> {
        self.0.member
    }

    /// The subsystem's name, for kind [`ErrorKind::Subsystem`].
    #[must_use]
    pub fn subsystem(&self) -> Option<&str> {
        self.0.subsystem.as_deref()
    }

    /// What the operator or the caller should do.
    #[must_use]
    pub fn action(&self) -> &str {
        &self.0.action
    }

    /// The process exit code for this error's kind (see [`exit`]).
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.0.kind.exit_value())
    }

    /// Writes the error to standard error as one block (what failed, at
    /// which step, and what to do) and returns its exit code.
    ///
    /// Use it in `main` when [`start!`](crate::start) fails: a failure
    /// before logging opens has nowhere else to go. It writes with one
    /// `write_all` and never panics; if standard error is closed the write
    /// fails silently and the exit code is still returned, because there is
    /// nowhere left to report it.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[cfg(feature = "kernel")]
    /// fn main() -> std::process::ExitCode {
    ///     let system = match system_suite::start!() {
    ///         Ok(system) => system,
    ///         Err(error) => return error.report(),
    ///     };
    ///     system.run()
    /// }
    /// # #[cfg(not(feature = "kernel"))]
    /// # fn main() {}
    /// ```
    pub fn report(&self) -> ExitCode {
        let text = self.report_text();
        let mut stderr = io::stderr().lock();
        // Nothing can be done if stderr itself fails; the exit code still
        // carries the failure.
        drop(stderr.write_all(text.as_bytes()));
        drop(stderr.flush());
        self.exit_code()
    }

    fn report_text(&self) -> String {
        let app = self.0.app.as_deref().unwrap_or("system-suite");
        format!("{app}: {self}\n  do: {}\n", self.0.action)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.step {
            Some(step) => write!(f, "boot {step} failed: {}", self.0.detail),
            None => f.write_str(&self.0.detail),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("kind", &self.0.kind)
            .field("step", &self.0.step)
            .field("member", &self.0.member)
            .field("subsystem", &self.0.subsystem)
            .field("detail", &self.0.detail)
            .field("action", &self.0.action)
            .field("source", &self.0.source)
            .finish()
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.0
            .source
            .as_deref()
            .map(|source| source as &(dyn StdError + 'static))
    }
}

/// The kinds of [`Error`]. Each has an exit code (see [`exit`]) and a
/// documented action.
///
/// # Examples
///
/// ```
/// use system_suite::ErrorKind;
///
/// let kind = ErrorKind::AlreadyRunning;
/// assert_eq!(kind, ErrorKind::AlreadyRunning);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// An invalid builder setting: a bad name, a member that cannot be
    /// disabled, a zero deadline. Fix the code; the message names the
    /// setting. Exit code 64.
    Setup,
    /// An application directory cannot be resolved or created, or is a
    /// symlink. The message names the directory; set `<NAME>_<KIND>_DIR` or
    /// fix the permissions. Exit code 73.
    Paths,
    /// The `.env` file is missing (when named explicitly), malformed, too
    /// large, or unreadable. Fix the line named in the message. Exit code 78.
    Environment,
    /// Another copy holds the single-instance lock. Stop the other copy, or
    /// use another name. Exit code 75.
    AlreadyRunning,
    /// The lock directory cannot be created, is writable by others, is a
    /// symlink, or is on a network filesystem. The message gives the fix.
    /// Exit code 77.
    LockDirInsecure,
    /// The hard open-file limit is below the minimum. Raise `LimitNOFILE=`
    /// in the systemd unit, `ulimit -Hn`, or the launchd limit. Exit code 69.
    Limits,
    /// The signal intake could not be installed. The message names the
    /// cause; another library may own the signal, or disable
    /// [`Member::Signals`]. Exit code 70.
    Signals,
    /// A log sink cannot be opened. The message names the file and the I/O
    /// error. Exit code 73.
    Logging,
    /// A kernel thread could not be started. Check the process and thread
    /// limits. Exit code 70.
    Kernel,
    /// A stop signal arrived during boot and the boot unwound cleanly.
    /// Nothing to do: the operator asked for it. Exit code 0.
    Interrupted,
    /// An I/O error not covered above. The message names the operation and
    /// the file. Exit code 74.
    Io,
}

impl ErrorKind {
    /// The exit code for this kind.
    pub(crate) const fn exit_value(self) -> u8 {
        match self {
            ErrorKind::Setup => exit::USAGE,
            ErrorKind::Paths | ErrorKind::Logging => exit::CANTCREAT,
            ErrorKind::Environment => exit::CONFIG,
            ErrorKind::AlreadyRunning => exit::TEMPFAIL,
            ErrorKind::LockDirInsecure => exit::NOPERM,
            ErrorKind::Limits => exit::UNAVAILABLE,
            ErrorKind::Signals | ErrorKind::Kernel => exit::SOFTWARE,
            ErrorKind::Interrupted => exit::OK,
            ErrorKind::Io => exit::IOERR,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_names_step_and_detail() {
        let error = Error::new(
            ErrorKind::AlreadyRunning,
            "another copy is running",
            "stop it",
        )
        .at(Step::Guard);
        assert_eq!(
            error.to_string(),
            "boot step 6 (guard) failed: another copy is running"
        );
        assert_eq!(error.member(), Some(Member::Sys));
        assert_eq!(error.action(), "stop it");
    }

    #[test]
    fn test_error_at_keeps_the_first_step() {
        let error = Error::new(ErrorKind::Io, "x", "y")
            .at(Step::Logging)
            .at(Step::Ready);
        assert_eq!(error.step(), Some(Step::Logging));
    }

    #[test]
    fn test_error_without_step_displays_detail_only() {
        let error = Error::new(ErrorKind::Setup, "name is empty", "give it a name");
        assert_eq!(error.to_string(), "name is empty");
        assert_eq!(error.step(), None);
    }

    #[test]
    fn test_error_report_text_names_app_and_action() {
        let error = Error::new(ErrorKind::Paths, "cannot create /x", "fix it")
            .at(Step::Directories)
            .for_app("site");
        assert_eq!(
            error.report_text(),
            "site: boot step 9 (directories) failed: cannot create /x\n  do: fix it\n"
        );
    }

    #[test]
    fn test_error_source_is_kept() {
        let io = io::Error::other("disk gone");
        let error = Error::new(ErrorKind::Io, "write failed", "check the disk").caused_by(io);
        let source = error.source().map(ToString::to_string);
        assert_eq!(source.as_deref(), Some("disk gone"));
    }

    #[test]
    fn test_error_kind_exit_codes_follow_sysexits() {
        assert_eq!(ErrorKind::Setup.exit_value(), 64);
        assert_eq!(ErrorKind::Paths.exit_value(), 73);
        assert_eq!(ErrorKind::Environment.exit_value(), 78);
        assert_eq!(ErrorKind::AlreadyRunning.exit_value(), 75);
        assert_eq!(ErrorKind::LockDirInsecure.exit_value(), 77);
        assert_eq!(ErrorKind::Limits.exit_value(), 69);
        assert_eq!(ErrorKind::Signals.exit_value(), 70);
        assert_eq!(ErrorKind::Logging.exit_value(), 73);
        assert_eq!(ErrorKind::Kernel.exit_value(), 70);
        assert_eq!(ErrorKind::Interrupted.exit_value(), 0);
        assert_eq!(ErrorKind::Io.exit_value(), 74);
    }

    #[test]
    fn test_error_exit_code_matches_kind() {
        let error = Error::new(ErrorKind::AlreadyRunning, "x", "y");
        assert_eq!(error.exit_code(), ExitCode::from(exit::TEMPFAIL));
    }

    #[test]
    fn test_error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<Error>();
    }
}
