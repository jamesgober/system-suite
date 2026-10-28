//! Health: per component, aggregated, and reported.

use std::{borrow::Cow, fmt};

use super::Level;
use crate::{Member, lifecycle::Phase};

/// How well something is working. Values compare by severity.
///
/// # Examples
///
/// ```
/// use system_suite::kernel::Status;
///
/// assert!(Status::Unhealthy > Status::Degraded);
/// assert_eq!(Status::Healthy.to_string(), "healthy");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Status {
    /// Working as intended.
    Healthy,
    /// Working, with a problem worth a look.
    Degraded,
    /// Not working.
    Unhealthy,
}

impl Status {
    /// The status's name, as used in reports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Status::Healthy => "healthy",
            Status::Degraded => "degraded",
            Status::Unhealthy => "unhealthy",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One component's health: a status and, when not healthy, a reason.
///
/// # Examples
///
/// ```
/// use system_suite::kernel::{Health, Status};
///
/// let health = Health::degraded("replica lag 12 s");
/// assert_eq!(health.status(), Status::Degraded);
/// assert_eq!(health.reason(), Some("replica lag 12 s"));
/// assert_eq!(Health::HEALTHY.reason(), None);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Health {
    status: Status,
    reason: Option<Cow<'static, str>>,
}

impl Health {
    /// Healthy, with no reason.
    pub const HEALTHY: Health = Health {
        status: Status::Healthy,
        reason: None,
    };

    /// Degraded, with a reason.
    pub fn degraded(reason: impl Into<Cow<'static, str>>) -> Health {
        Health {
            status: Status::Degraded,
            reason: Some(reason.into()),
        }
    }

    /// Unhealthy, with a reason.
    pub fn unhealthy(reason: impl Into<Cow<'static, str>>) -> Health {
        Health {
            status: Status::Unhealthy,
            reason: Some(reason.into()),
        }
    }

    /// The status.
    #[must_use]
    pub fn status(&self) -> Status {
        self.status
    }

    /// The reason, when there is one.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

/// What a [`ComponentHealth`] describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Component {
    /// A member of the suite.
    Member(Member),
    /// An application subsystem.
    Subsystem,
    /// A supervised worker.
    Worker,
    /// The timer thread.
    Timers,
}

impl Component {
    fn kind_name(self) -> &'static str {
        match self {
            Component::Member(_) => "member",
            Component::Subsystem => "subsystem",
            Component::Worker => "worker",
            Component::Timers => "timers",
        }
    }
}

/// One line of a [`HealthReport`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ComponentHealth {
    /// What it is.
    pub kind: Component,
    /// Its name.
    pub name: Cow<'static, str>,
    /// Its health.
    pub health: Health,
}

/// The health of the whole system: every member, subsystem, worker, and the
/// timer thread (FR-11).
///
/// The aggregate is the worst component status, except that a worker that
/// is not critical can make the system at most `Degraded` (D-022). Reasons
/// are the components' own words; the report never contains a configuration
/// value or a secret.
///
/// # Examples
///
/// ```
/// use system_suite::{Member, System};
///
/// # #[cfg(feature = "sys")] {
/// let system = System::builder("health-doc")
///     .process_wide(false)
///     .single_instance(false)
///     .disable(Member::Log)
///     .start()?;
/// let report = system.health();
/// assert!(report.ready());
/// let mut json = String::new();
/// report.write_json(&mut json).ok();
/// assert!(json.starts_with("{\"status\":\"healthy\""));
/// let code = system.shutdown();
/// # let _ = code;
/// # }
/// # Ok::<(), system_suite::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct HealthReport {
    pub(crate) status: Status,
    pub(crate) phase: Phase,
    pub(crate) urgent: Level,
    pub(crate) components: Vec<ComponentHealth>,
}

impl HealthReport {
    /// The aggregate status.
    #[must_use]
    pub fn status(&self) -> Status {
        self.status
    }

    /// The lifecycle phase when the report was taken.
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The urgent line's level when the report was taken.
    #[must_use]
    pub fn urgent(&self) -> Level {
        self.urgent
    }

    /// For a readiness probe: the phase is `Ready` and the status is not
    /// `Unhealthy`.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.phase == Phase::Ready && self.status != Status::Unhealthy
    }

    /// For a liveness probe: the kernel is running (any phase before
    /// `Stopped`).
    #[must_use]
    pub fn live(&self) -> bool {
        self.phase < Phase::Stopped
    }

    /// Every component's health.
    #[must_use]
    pub fn components(&self) -> &[ComponentHealth] {
        &self.components
    }

    /// The report as one JSON object.
    ///
    /// # Errors
    ///
    /// Returns the error `out` returns.
    pub fn write_json(&self, out: &mut impl fmt::Write) -> fmt::Result {
        write!(
            out,
            "{{\"status\":\"{}\",\"phase\":\"{}\",\"urgent\":\"{}\",\"components\":[",
            self.status, self.phase, self.urgent
        )?;
        for (index, component) in self.components.iter().enumerate() {
            if index > 0 {
                out.write_char(',')?;
            }
            write!(
                out,
                "{{\"kind\":\"{}\",\"name\":",
                component.kind.kind_name()
            )?;
            write_json_string(out, &component.name)?;
            write!(out, ",\"status\":\"{}\"", component.health.status)?;
            if let Some(reason) = component.health.reason() {
                out.write_str(",\"reason\":")?;
                write_json_string(out, reason)?;
            }
            out.write_char('}')?;
        }
        out.write_str("]}")
    }

    /// The report as `key=value` lines: one for the system, one per
    /// component.
    ///
    /// # Errors
    ///
    /// Returns the error `out` returns.
    pub fn write_kv(&self, out: &mut impl fmt::Write) -> fmt::Result {
        writeln!(
            out,
            "health.status={} health.phase={} health.urgent={}",
            self.status, self.phase, self.urgent
        )?;
        for component in &self.components {
            write!(
                out,
                "health.{}.{}={}",
                component.kind.kind_name(),
                component.name,
                component.health.status
            )?;
            if let Some(reason) = component.health.reason() {
                write!(out, " reason={reason:?}")?;
            }
            out.write_char('\n')?;
        }
        Ok(())
    }
}

impl fmt::Display for HealthReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_kv(f)
    }
}

/// Writes `text` as a JSON string, escaping what JSON requires.
fn write_json_string(out: &mut impl fmt::Write, text: &str) -> fmt::Result {
    out.write_char('"')?;
    for c in text.chars() {
        match c {
            '"' => out.write_str("\\\"")?,
            '\\' => out.write_str("\\\\")?,
            '\n' => out.write_str("\\n")?,
            '\r' => out.write_str("\\r")?,
            '\t' => out.write_str("\\t")?,
            c if u32::from(c) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                write!(out, "\\u{:04x}", u32::from(c))?;
            }
            c => out.write_char(c)?,
        }
    }
    out.write_char('"')
}

/// The aggregate of `components`: the worst status, with non-critical
/// workers capped at `Degraded` by the caller.
pub(crate) fn aggregate<'a>(statuses: impl Iterator<Item = Status> + 'a) -> Status {
    statuses.max().unwrap_or(Status::Healthy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(components: Vec<ComponentHealth>) -> HealthReport {
        let status = aggregate(components.iter().map(|c| c.health.status()));
        HealthReport {
            status,
            phase: Phase::Ready,
            urgent: Level::Normal,
            components,
        }
    }

    fn component(name: &'static str, health: Health) -> ComponentHealth {
        ComponentHealth {
            kind: Component::Subsystem,
            name: Cow::Borrowed(name),
            health,
        }
    }

    #[test]
    fn test_aggregate_is_the_worst_status() {
        assert_eq!(aggregate(std::iter::empty()), Status::Healthy);
        let r = report(vec![
            component("a", Health::HEALTHY),
            component("b", Health::degraded("slow")),
        ]);
        assert_eq!(r.status(), Status::Degraded);
        assert!(r.ready());
        let r = report(vec![component("c", Health::unhealthy("down"))]);
        assert!(!r.ready());
        assert!(r.live());
    }

    #[test]
    fn test_write_json_escapes_reasons() {
        let r = report(vec![component("db", Health::degraded("say \"hi\"\n\u{1}"))]);
        let mut json = String::new();
        assert!(r.write_json(&mut json).is_ok());
        assert_eq!(
            json,
            "{\"status\":\"degraded\",\"phase\":\"ready\",\"urgent\":\"normal\",\"components\":[{\"kind\":\"subsystem\",\"name\":\"db\",\"status\":\"degraded\",\"reason\":\"say \\\"hi\\\"\\n\\u0001\"}]}"
        );
    }

    #[test]
    fn test_write_kv_has_one_line_per_component() {
        let r = report(vec![
            component("a", Health::HEALTHY),
            component("b", Health::degraded("slow")),
        ]);
        let text = r.to_string();
        assert_eq!(text.lines().count(), 3);
        assert!(text.contains("health.subsystem.b=degraded reason=\"slow\""));
    }

    #[test]
    fn test_stopped_is_neither_live_nor_ready() {
        let mut r = report(Vec::new());
        r.phase = Phase::Stopped;
        assert!(!r.live());
        assert!(!r.ready());
    }
}
