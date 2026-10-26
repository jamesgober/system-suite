//! The phases of the system, why it stops, and the deadlines it stops by,
//! and the contract every application subsystem follows ([`Subsystem`]).
//!
//! The kernel drives the lifecycle.

use std::{
    fmt,
    time::{Duration, Instant},
};

mod subsystem;

pub use subsystem::{Commit, Failure, Link, Mailbox, Position, Subsystem};

/// The phase of the system's lifecycle.
///
/// ```text
///  Booting ─▶ Ready ─▶ Draining ─▶ Stopping ─▶ Stopped
///     │                    ▲
///     └── boot failure ────┘ (reverse shutdown of what started, then Stopped)
/// ```
///
/// Values compare in lifecycle order, so `phase >= Phase::Draining` means
/// "going down". A reload never changes the phase.
///
/// # Examples
///
/// ```
/// use system_suite::lifecycle::Phase;
///
/// assert!(Phase::Draining > Phase::Ready);
/// assert_eq!(Phase::Ready.to_string(), "ready");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum Phase {
    /// The boot order is running; `start` has not returned.
    Booting = 0,
    /// Every step succeeded; the system is serving.
    Ready = 1,
    /// A stop was requested; in-flight work finishes, no new work starts.
    Draining = 2,
    /// The members are being stopped.
    Stopping = 3,
    /// Everything stopped; `run` or `shutdown` is returning.
    Stopped = 4,
}

impl Phase {
    /// The phase's name, as used in logs and health reports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Phase::Booting => "booting",
            Phase::Ready => "ready",
            Phase::Draining => "draining",
            Phase::Stopping => "stopping",
            Phase::Stopped => "stopped",
        }
    }

    pub(crate) const fn from_u8(value: u8) -> Phase {
        match value {
            0 => Phase::Booting,
            1 => Phase::Ready,
            2 => Phase::Draining,
            3 => Phase::Stopping,
            _ => Phase::Stopped,
        }
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Why the system stops.
///
/// The first reason wins: a second stop request while one is in progress is
/// recorded in the log and does not replace it.
///
/// # Examples
///
/// ```
/// use system_suite::lifecycle::StopReason;
///
/// let reason = StopReason::Requested { reason: "admin" };
/// assert_eq!(reason.exit_code(), 0);
/// assert_eq!(reason.to_string(), "requested: admin");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StopReason {
    /// An OS signal or console event: `"SIGTERM"`, `"SIGINT"`, `"CTRL_C"`,
    /// `"CTRL_CLOSE"`, ...
    Signal {
        /// The signal's conventional name.
        name: &'static str,
    },
    /// The application asked (`Kernel::request_stop`).
    Requested {
        /// The reason the application gave.
        reason: &'static str,
    },
    /// A critical worker exhausted its restart budget (from `0.4`).
    WorkerFailed {
        /// The worker's name.
        name: String,
    },
    /// The out-of-process watchdog asked, when one is present.
    Supervisor {
        /// What the supervisor said.
        reason: String,
    },
}

impl StopReason {
    /// The exit code a stop for this reason ends with: `0`, except
    /// `WorkerFailed`, which is `70` (`exit::SOFTWARE`).
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            StopReason::WorkerFailed { .. } => crate::exit::SOFTWARE,
            _ => crate::exit::OK,
        }
    }
}

impl fmt::Display for StopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StopReason::Signal { name } => write!(f, "signal {name}"),
            StopReason::Requested { reason } => write!(f, "requested: {reason}"),
            StopReason::WorkerFailed { name } => {
                write!(f, "critical worker {name} exhausted its restarts")
            }
            StopReason::Supervisor { reason } => write!(f, "supervisor: {reason}"),
        }
    }
}

/// The point in time by which a shutdown step must finish.
///
/// Always the OS monotonic clock, never a test clock, so a manual clock can
/// never stall a shutdown (D-029).
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use system_suite::lifecycle::Deadline;
///
/// let deadline = Deadline::after(Duration::from_millis(50));
/// assert!(!deadline.is_past());
/// assert!(deadline.remaining() <= Duration::from_millis(50));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Deadline(Instant);

impl Deadline {
    /// A deadline `duration` from now. A duration too large to represent is
    /// capped about 34 years out.
    #[must_use]
    pub fn after(duration: Duration) -> Deadline {
        let now = Instant::now();
        let cap = Duration::from_secs(1 << 30);
        Deadline(
            now.checked_add(duration.min(cap))
                .or_else(|| now.checked_add(Duration::from_secs(1)))
                .unwrap_or(now),
        )
    }

    pub(crate) fn at(instant: Instant) -> Deadline {
        Deadline(instant)
    }

    /// The deadline as an `Instant`.
    #[must_use]
    pub fn instant(&self) -> Instant {
        self.0
    }

    /// The time left; zero once the deadline has passed.
    #[must_use]
    pub fn remaining(&self) -> Duration {
        self.0.saturating_duration_since(Instant::now())
    }

    /// Whether the deadline has passed.
    #[must_use]
    pub fn is_past(&self) -> bool {
        Instant::now() >= self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phase_round_trips_through_u8() {
        for phase in [
            Phase::Booting,
            Phase::Ready,
            Phase::Draining,
            Phase::Stopping,
            Phase::Stopped,
        ] {
            assert_eq!(Phase::from_u8(phase as u8), phase);
        }
        assert_eq!(Phase::from_u8(200), Phase::Stopped);
    }

    #[test]
    fn test_phase_orders_by_lifecycle() {
        assert!(Phase::Booting < Phase::Ready);
        assert!(Phase::Stopping < Phase::Stopped);
    }

    #[test]
    fn test_stop_reason_exit_codes() {
        assert_eq!(StopReason::Signal { name: "SIGTERM" }.exit_code(), 0);
        assert_eq!(
            StopReason::WorkerFailed {
                name: "jobs".to_owned()
            }
            .exit_code(),
            70
        );
        assert_eq!(
            StopReason::Signal { name: "SIGTERM" }.to_string(),
            "signal SIGTERM"
        );
    }

    #[test]
    fn test_deadline_zero_is_past() {
        let deadline = Deadline::after(Duration::ZERO);
        assert!(deadline.is_past());
        assert_eq!(deadline.remaining(), Duration::ZERO);
    }

    #[test]
    fn test_deadline_huge_duration_does_not_panic() {
        let deadline = Deadline::after(Duration::MAX);
        assert!(!deadline.is_past());
    }
}
