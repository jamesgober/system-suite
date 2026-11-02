//! The urgent line's memory watermark: `Shed` under memory pressure, and
//! back to `Normal` once it has passed (D-081). Off unless `[kernel]
//! memory_watch` is set, because every reading wakes a thread.

use sys_lib::PressureLevel;

use super::{Level, UrgentLine};

/// Readings in a row below the mark before the line is cleared.
const CALM_READINGS: u8 = 3;
/// How far below the mark memory has to fall, in percentage points.
const CLEAR_MARGIN: f32 = 5.0;

const RAISE_REASON: &str = "memory above the watermark";
const CLEAR_REASON: &str = "memory back below the watermark";

/// One look at the machine's memory (the container's, inside one).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Reading {
    /// `None` on a host that does not report pressure.
    pub(crate) pressure: Option<PressureLevel>,
    pub(crate) used_percent: f32,
}

/// Reads the memory state. The share used is the floor every host has; a
/// failed pressure reading falls back to it.
pub(crate) fn read() -> Result<Reading, sys_lib::Error> {
    Ok(Reading {
        pressure: sys_lib::mem::pressure().ok().flatten().map(|p| p.level),
        used_percent: sys_lib::mem::used_percent()?.value(),
    })
}

/// The watermark's state between readings.
pub(crate) struct Watermark {
    shed_percent: f32,
    /// The epoch of the change this watermark made, while it holds the
    /// line at `Shed`.
    held: Option<u64>,
    calm: u8,
}

impl Watermark {
    pub(crate) fn new(shed_percent: u8) -> Watermark {
        Watermark {
            shed_percent: f32::from(shed_percent),
            held: None,
            calm: 0,
        }
    }

    /// Acts on one reading. Only ever moves the line from `Normal` to
    /// `Shed` and back: it never lowers a `Lockdown`, and a level someone
    /// else set stays theirs to clear.
    pub(crate) fn step(&mut self, reading: Reading, line: &UrgentLine) {
        let pressed = reading
            .pressure
            .is_some_and(|level| level >= PressureLevel::Warning)
            || reading.used_percent >= self.shed_percent;
        let Some(epoch) = self.held else {
            if pressed {
                self.held = line.shift(Level::Normal, Level::Shed, RAISE_REASON);
            }
            return;
        };
        if line.epoch() != epoch {
            self.held = None;
            self.calm = 0;
            return;
        }
        let calm = !pressed && reading.used_percent < self.shed_percent - CLEAR_MARGIN;
        self.calm = if calm { self.calm + 1 } else { 0 };
        if self.calm >= CALM_READINGS {
            let _cleared = line.shift(Level::Shed, Level::Normal, CLEAR_REASON);
            self.held = None;
            self.calm = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HIGH: Reading = Reading {
        pressure: None,
        used_percent: 95.0,
    };
    const CALM: Reading = Reading {
        pressure: Some(PressureLevel::Normal),
        used_percent: 40.0,
    };

    #[test]
    fn test_the_share_used_raises_and_three_calm_readings_clear() {
        let line = UrgentLine::detached();
        let mut mark = Watermark::new(90);
        mark.step(CALM, &line);
        assert_eq!(line.level(), Level::Normal);
        mark.step(HIGH, &line);
        assert_eq!(line.level(), Level::Shed);
        assert_eq!(line.notice().reason, RAISE_REASON);
        mark.step(CALM, &line);
        mark.step(CALM, &line);
        assert_eq!(line.level(), Level::Shed);
        mark.step(CALM, &line);
        assert_eq!(line.level(), Level::Normal);
        assert_eq!(line.notice().reason, CLEAR_REASON);
    }

    #[test]
    fn test_pressure_raises_with_memory_to_spare() {
        for level in [PressureLevel::Warning, PressureLevel::Critical] {
            let line = UrgentLine::detached();
            let mut mark = Watermark::new(90);
            let reading = Reading {
                pressure: Some(level),
                used_percent: 30.0,
            };
            mark.step(reading, &line);
            assert_eq!(line.level(), Level::Shed);
        }
    }

    #[test]
    fn test_a_reading_near_the_mark_does_not_flap() {
        let line = UrgentLine::detached();
        let mut mark = Watermark::new(90);
        mark.step(HIGH, &line);
        // Below the mark but inside the margin: not calm.
        let near = Reading {
            pressure: Some(PressureLevel::Normal),
            used_percent: 87.0,
        };
        for _ in 0..10 {
            mark.step(near, &line);
        }
        assert_eq!(line.level(), Level::Shed);
        assert_eq!(line.epoch(), 1, "the line moved");
        // A high reading restarts the count.
        mark.step(CALM, &line);
        mark.step(CALM, &line);
        mark.step(HIGH, &line);
        mark.step(CALM, &line);
        mark.step(CALM, &line);
        assert_eq!(line.level(), Level::Shed);
    }

    #[test]
    fn test_a_lockdown_is_never_lowered_nor_taken_over() {
        let line = UrgentLine::detached();
        let mut mark = Watermark::new(90);
        assert!(line.set(Level::Lockdown, "breach").is_ok());
        mark.step(HIGH, &line);
        assert_eq!(line.level(), Level::Lockdown);
        for _ in 0..5 {
            mark.step(CALM, &line);
        }
        assert_eq!(line.level(), Level::Lockdown);

        // A lockdown that arrives while the watermark holds the line.
        let line = UrgentLine::detached();
        let mut mark = Watermark::new(90);
        mark.step(HIGH, &line);
        assert!(line.set(Level::Lockdown, "breach").is_ok());
        for _ in 0..5 {
            mark.step(CALM, &line);
        }
        assert_eq!(line.level(), Level::Lockdown);
        assert_eq!(line.notice().reason, "breach");
    }

    #[test]
    fn test_a_shed_set_by_someone_else_stays_theirs() {
        let line = UrgentLine::detached();
        let mut mark = Watermark::new(90);
        assert!(line.set(Level::Shed, "supervisor").is_ok());
        mark.step(HIGH, &line);
        for _ in 0..5 {
            mark.step(CALM, &line);
        }
        assert_eq!(line.level(), Level::Shed);
        assert_eq!(line.notice().reason, "supervisor");
    }

    #[test]
    fn test_a_stop_is_never_undone() {
        let line = UrgentLine::detached();
        let mut mark = Watermark::new(90);
        mark.step(HIGH, &line);
        assert!(line.raise(Level::Draining, "stop"));
        for _ in 0..5 {
            mark.step(CALM, &line);
        }
        assert_eq!(line.level(), Level::Draining);
        let line = UrgentLine::detached();
        assert!(line.raise(Level::Stopping, "stop"));
        let mut mark = Watermark::new(90);
        mark.step(HIGH, &line);
        assert_eq!(line.level(), Level::Stopping);
    }

    #[test]
    fn test_read_reports_a_share_on_this_machine() {
        let reading = read().unwrap_or_else(|e| panic!("{e}"));
        assert!(reading.used_percent > 0.0 && reading.used_percent <= 100.0);
    }
}
