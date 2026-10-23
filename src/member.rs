//! The members of the suite and the steps of the boot order.

use std::fmt;

/// A member of the suite: one published library behind one feature, or the
/// kernel itself.
///
/// Names a member in errors, in health reports, and in
/// [`Builder::disable`](crate::Builder::disable).
///
/// # Examples
///
/// ```
/// use system_suite::Member;
///
/// assert_eq!(Member::Log.name(), "log");
/// assert_eq!(Member::Log.crate_name(), "log-io");
/// assert_eq!(Member::Kernel.is_compiled(), cfg!(feature = "kernel"));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Member {
    /// The kernel, built in this crate (feature `kernel`).
    Kernel,
    /// System facts, build info, instance info, process guard (sys-lib, feature `sys`).
    Sys,
    /// Application and OS directories (path-lib, feature `paths`).
    Paths,
    /// Environment variables and the `.env` file (env-lib, feature `env`).
    Env,
    /// Configuration (config-lib, feature `config`).
    Config,
    /// Structured logging (log-io, feature `log`).
    Log,
    /// Metrics (metrics-lib, feature `metrics`).
    Metrics,
    /// OS signals (signal-mod, feature `signals`).
    Signals,
    /// Monotonic and wall clocks (clock-lib, feature `time`).
    Time,
    /// Ids (id-forge, feature `ids`).
    Ids,
    /// Tamper-evident audit records (audit-trail, feature `audit`).
    Audit,
    /// Kernel events delivered to callbacks (registry-io, feature `registry`).
    Registry,
    /// Process-wide typed values (global-lib, feature `globals`).
    Globals,
    /// Randomness (mod-rand, feature `rand`).
    Rand,
    /// Secret storage (key-vault, feature `secrets`).
    Secrets,
}

impl Member {
    /// The feature name, which is also the member's name in messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Member::Kernel => "kernel",
            Member::Sys => "sys",
            Member::Paths => "paths",
            Member::Env => "env",
            Member::Config => "config",
            Member::Log => "log",
            Member::Metrics => "metrics",
            Member::Signals => "signals",
            Member::Time => "time",
            Member::Ids => "ids",
            Member::Audit => "audit",
            Member::Registry => "registry",
            Member::Globals => "globals",
            Member::Rand => "rand",
            Member::Secrets => "secrets",
        }
    }

    /// The published crate that provides the member (`"system-suite"` for
    /// the kernel).
    #[must_use]
    pub const fn crate_name(self) -> &'static str {
        match self {
            Member::Kernel => "system-suite",
            Member::Sys => "sys-lib",
            Member::Paths => "path-lib",
            Member::Env => "env-lib",
            Member::Config => "config-lib",
            Member::Log => "log-io",
            Member::Metrics => "metrics-lib",
            Member::Signals => "signal-mod",
            Member::Time => "clock-lib",
            Member::Ids => "id-forge",
            Member::Audit => "audit-trail",
            Member::Registry => "registry-io",
            Member::Globals => "global-lib",
            Member::Rand => "mod-rand",
            Member::Secrets => "key-vault",
        }
    }

    /// Whether the member's feature is enabled in this build.
    #[must_use]
    pub const fn is_compiled(self) -> bool {
        match self {
            Member::Kernel => cfg!(feature = "kernel"),
            Member::Sys => cfg!(feature = "sys"),
            Member::Paths => cfg!(feature = "paths"),
            Member::Env => cfg!(feature = "env"),
            Member::Config => cfg!(feature = "config"),
            Member::Log => cfg!(feature = "log"),
            Member::Metrics => cfg!(feature = "metrics"),
            Member::Signals => cfg!(feature = "signals"),
            Member::Time => cfg!(feature = "time"),
            Member::Ids => cfg!(feature = "ids"),
            Member::Audit => cfg!(feature = "audit"),
            Member::Registry => cfg!(feature = "registry"),
            Member::Globals => cfg!(feature = "globals"),
            Member::Rand => cfg!(feature = "rand"),
            Member::Secrets => cfg!(feature = "secrets"),
        }
    }

    /// Whether [`Builder::disable`](crate::Builder::disable) accepts the
    /// member. Members that others need at run time, or that hold no
    /// run-time state to turn off, cannot be disabled.
    pub(crate) const fn can_disable(self) -> bool {
        matches!(
            self,
            Member::Env
                | Member::Config
                | Member::Log
                | Member::Metrics
                | Member::Signals
                | Member::Audit
                | Member::Registry
                | Member::Globals
                | Member::Secrets
        )
    }
}

impl fmt::Display for Member {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A step of the boot order, as documented in `docs/BOOT-ORDER.md`.
///
/// Values compare in boot order. An error raised during boot names the step
/// it happened in.
///
/// # Examples
///
/// ```
/// use system_suite::Step;
///
/// assert!(Step::Configuration < Step::Guard);
/// assert_eq!(Step::Guard.number(), 6);
/// assert_eq!(Step::Guard.to_string(), "step 6 (guard)");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Step {
    /// 1: the application's build info is recorded.
    BuildInfo,
    /// 2: the instance (id, start time, host, pid, user) is created.
    Instance,
    /// 3: the application's directories are resolved; nothing is created.
    Paths,
    /// 4: the `.env` file is read.
    Environment,
    /// 5: the configuration file is found, checked, parsed, and validated.
    Configuration,
    /// 6: the single-instance lock, umask, and open-file limit.
    Guard,
    /// 7: the kernel's signal intake is installed.
    Signals,
    /// 8: secret references are resolved into the vault.
    Secrets,
    /// 9: the directories the enabled members write to are created.
    Directories,
    /// 10: log sinks are opened.
    Logging,
    /// 11: the panic hook is installed.
    CrashCapture,
    /// 12: the metrics registry is created.
    Metrics,
    /// 13: the audit chain is opened, verified, and resumed.
    Audit,
    /// 14: the kernel event registry is created.
    Registry,
    /// 15: the kernel starts.
    Kernel,
    /// 16: the application's subsystems start.
    Subsystems,
    /// 17: the system is ready.
    Ready,
}

impl Step {
    /// The position of the step in the boot order, 1 to 17.
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Step::BuildInfo => 1,
            Step::Instance => 2,
            Step::Paths => 3,
            Step::Environment => 4,
            Step::Configuration => 5,
            Step::Guard => 6,
            Step::Signals => 7,
            Step::Secrets => 8,
            Step::Directories => 9,
            Step::Logging => 10,
            Step::CrashCapture => 11,
            Step::Metrics => 12,
            Step::Audit => 13,
            Step::Registry => 14,
            Step::Kernel => 15,
            Step::Subsystems => 16,
            Step::Ready => 17,
        }
    }

    /// The step's short name, as used in messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Step::BuildInfo => "build info",
            Step::Instance => "instance",
            Step::Paths => "paths",
            Step::Environment => "environment",
            Step::Configuration => "configuration",
            Step::Guard => "guard",
            Step::Signals => "signals",
            Step::Secrets => "secrets",
            Step::Directories => "directories",
            Step::Logging => "logging",
            Step::CrashCapture => "crash capture",
            Step::Metrics => "metrics",
            Step::Audit => "audit",
            Step::Registry => "registry",
            Step::Kernel => "kernel",
            Step::Subsystems => "subsystems",
            Step::Ready => "ready",
        }
    }

    /// The member that performs the step, if it is one member's.
    #[must_use]
    pub const fn member(self) -> Option<Member> {
        match self {
            Step::BuildInfo | Step::Instance | Step::Guard => Some(Member::Sys),
            Step::Paths | Step::Directories => Some(Member::Paths),
            Step::Environment => Some(Member::Env),
            Step::Configuration => Some(Member::Config),
            Step::Signals => Some(Member::Signals),
            Step::Secrets => Some(Member::Secrets),
            Step::Logging => Some(Member::Log),
            Step::Metrics => Some(Member::Metrics),
            Step::Audit => Some(Member::Audit),
            Step::Registry => Some(Member::Registry),
            Step::CrashCapture | Step::Kernel | Step::Ready => Some(Member::Kernel),
            Step::Subsystems => None,
        }
    }
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "step {} ({})", self.number(), self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEPS: [Step; 17] = [
        Step::BuildInfo,
        Step::Instance,
        Step::Paths,
        Step::Environment,
        Step::Configuration,
        Step::Guard,
        Step::Signals,
        Step::Secrets,
        Step::Directories,
        Step::Logging,
        Step::CrashCapture,
        Step::Metrics,
        Step::Audit,
        Step::Registry,
        Step::Kernel,
        Step::Subsystems,
        Step::Ready,
    ];

    #[test]
    fn test_step_numbers_follow_boot_order() {
        for (index, step) in STEPS.iter().enumerate() {
            assert_eq!(usize::from(step.number()), index + 1, "{step:?}");
        }
        assert!(STEPS.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn test_step_display_names_number_and_name() {
        assert_eq!(Step::BuildInfo.to_string(), "step 1 (build info)");
        assert_eq!(Step::Ready.to_string(), "step 17 (ready)");
    }

    #[test]
    fn test_member_names_are_feature_names() {
        assert_eq!(Member::Paths.name(), "paths");
        assert_eq!(Member::Signals.crate_name(), "signal-mod");
        assert_eq!(Member::Kernel.to_string(), "kernel");
    }

    #[test]
    fn test_member_can_disable_matches_documented_table() {
        assert!(Member::Log.can_disable());
        assert!(Member::Signals.can_disable());
        assert!(!Member::Kernel.can_disable());
        assert!(!Member::Sys.can_disable());
        assert!(!Member::Paths.can_disable());
        assert!(!Member::Time.can_disable());
    }
}
