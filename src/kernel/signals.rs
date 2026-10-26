//! The kernel's signal intake (boot step 7).
//!
//! signal-mod's runtime-free back-end receives the OS signals; one parked
//! thread here turns each into a lifecycle decision. Signals reach only the
//! kernel (D-006).

use std::{
    sync::{Arc, OnceLock},
    thread::{self, JoinHandle},
};

use signal_mod::{Coordinator, Signal, SignalSet};

use super::Kernel;
use crate::{
    Error, ErrorKind, Member,
    lifecycle::StopReason,
    logging::{LogSlot, Val},
};

/// Writes the state dump requested by the dump signal.
pub(crate) type Dump = Box<dyn Fn() + Send + Sync + 'static>;

/// Reloads the configuration on the reload signal.
pub(crate) type Reload = Box<dyn Fn() + Send + Sync + 'static>;

/// What the reload and dump signals do. Set once the system is ready.
#[derive(Default)]
pub(crate) struct Actions {
    dump: OnceLock<Dump>,
    reload: OnceLock<Reload>,
    dump_on: bool,
    reload_on: bool,
}

/// The installed intake: the coordinator and its thread.
pub(crate) struct SignalIntake {
    coordinator: Coordinator,
    thread: Option<JoinHandle<()>>,
    actions: Arc<Actions>,
}

impl SignalIntake {
    /// Installs the intake. Stop signals request a stop; on Unix, SIGHUP
    /// (reload) and SIGUSR1 (dump) are received without stopping. With
    /// `[kernel] reload_signal` or `dump_signal` off they are still
    /// received (their default action would end the process) and ignored.
    pub(crate) fn install(
        kernel: Kernel,
        log: LogSlot,
        reload_on: bool,
        dump_on: bool,
    ) -> Result<SignalIntake, Error> {
        let (stop, notify) = signal_sets();
        let coordinator = Coordinator::builder()
            .signals(stop)
            .notify_signals(notify)
            .build();
        let mut subscription = coordinator.subscribe();
        coordinator.install().map_err(|e| {
            Error::new(
                ErrorKind::Signals,
                format!("the signal intake could not be installed: {e}"),
                "another library may own the signal; or start with disable(Member::Signals) and stop the process another way",
            )
            .member_of(Member::Signals)
            .caused_by(e)
        })?;
        let actions = Arc::new(Actions {
            reload_on,
            dump_on,
            ..Actions::default()
        });
        let thread_actions = Arc::clone(&actions);
        let spawned = thread::Builder::new()
            .name("system-suite-signals".to_owned())
            .spawn(move || {
                while let Some(signal) = subscription.recv_blocking() {
                    handle(signal, stop, &kernel, &log, &thread_actions);
                }
            });
        let thread = match spawned {
            Ok(thread) => thread,
            Err(e) => {
                let _removed = coordinator.uninstall();
                return Err(Error::new(
                    ErrorKind::Kernel,
                    format!("the signal intake thread could not be started: {e}"),
                    "check the process and thread limits",
                )
                .member_of(Member::Kernel)
                .caused_by(e));
            }
        };
        Ok(SignalIntake {
            coordinator,
            thread: Some(thread),
            actions,
        })
    }

    /// Sets what the dump signal writes. Only the first call counts.
    pub(crate) fn set_dump(&self, dump: Dump) {
        // A second dump writer is ignored; the first stays in force.
        drop(self.actions.dump.set(dump));
    }

    /// Sets what the reload signal does. Only the first call counts.
    #[cfg(feature = "config")]
    pub(crate) fn set_reload(&self, reload: Reload) {
        // A second reloader is ignored; the first stays in force.
        drop(self.actions.reload.set(reload));
    }

    /// Removes the intake. Signals then get the OS default action again.
    /// Returns the thread to join, which ends once the coordinator is gone.
    pub(crate) fn uninstall(mut self) -> Option<JoinHandle<()>> {
        let _was_installed = self.coordinator.uninstall();
        let thread = self.thread.take();
        // Dropping the coordinator closes the subscription, which ends the thread.
        drop(self);
        thread
    }
}

fn handle(signal: Signal, stop: SignalSet, kernel: &Kernel, log: &LogSlot, actions: &Actions) {
    let name = signal_name(signal);
    if stop.contains(signal) {
        if kernel.stop_for(StopReason::Signal { name }) {
            log.info("stop requested", &[("signal", Val::Str(name))]);
        } else {
            log.info(
                "stop signal ignored: already stopping",
                &[("signal", Val::Str(name))],
            );
        }
        return;
    }
    match signal {
        Signal::Hangup if !actions.reload_on => log.info(
            "reload signal ignored: [kernel] reload_signal is off",
            &[("signal", Val::Str(name))],
        ),
        Signal::Hangup => match actions.reload.get() {
            Some(reload) => {
                log.info("reload signal received", &[("signal", Val::Str(name))]);
                reload();
            }
            None => log.info(
                "reload signal ignored: the system is not ready or has no configuration",
                &[("signal", Val::Str(name))],
            ),
        },
        Signal::User1 if !actions.dump_on => log.info(
            "dump signal ignored: [kernel] dump_signal is off",
            &[("signal", Val::Str(name))],
        ),
        Signal::User1 => match actions.dump.get() {
            Some(write) => write(),
            None => log.info(
                "dump signal received during boot",
                &[("signal", Val::Str(name))],
            ),
        },
        _ => log.warn("unexpected signal ignored", &[("signal", Val::Str(name))]),
    }
}

/// The stop and notify sets for this OS.
///
/// On Windows signal-mod reports logoff and system shutdown as `Hangup`, so
/// there it stops the process; on Unix SIGHUP is the reload signal.
#[cfg(unix)]
fn signal_sets() -> (SignalSet, SignalSet) {
    let stop = SignalSet::empty()
        .with(Signal::Terminate)
        .with(Signal::Interrupt)
        .with(Signal::Quit);
    let notify = SignalSet::empty().with(Signal::Hangup).with(Signal::User1);
    (stop, notify)
}

#[cfg(windows)]
fn signal_sets() -> (SignalSet, SignalSet) {
    let stop = SignalSet::empty()
        .with(Signal::Terminate)
        .with(Signal::Interrupt)
        .with(Signal::Quit)
        .with(Signal::Hangup);
    (stop, SignalSet::empty())
}

#[cfg(not(any(unix, windows)))]
fn signal_sets() -> (SignalSet, SignalSet) {
    (SignalSet::empty(), SignalSet::empty())
}

/// The conventional name of a signal on this OS.
#[cfg(unix)]
fn signal_name(signal: Signal) -> &'static str {
    match signal {
        Signal::Terminate => "SIGTERM",
        Signal::Interrupt => "SIGINT",
        Signal::Quit => "SIGQUIT",
        Signal::Hangup => "SIGHUP",
        Signal::User1 => "SIGUSR1",
        Signal::User2 => "SIGUSR2",
        Signal::Pipe => "SIGPIPE",
        _ => "signal",
    }
}

#[cfg(not(unix))]
fn signal_name(signal: Signal) -> &'static str {
    match signal {
        Signal::Terminate => "CTRL_CLOSE",
        Signal::Interrupt => "CTRL_C",
        Signal::Quit => "CTRL_BREAK",
        Signal::Hangup => "CTRL_LOGOFF_OR_SHUTDOWN",
        _ => "console event",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_sets_stop_on_terminate_and_interrupt() {
        let (stop, notify) = signal_sets();
        assert!(stop.contains(Signal::Terminate));
        assert!(stop.contains(Signal::Interrupt));
        assert!(stop.contains(Signal::Quit));
        if cfg!(unix) {
            assert!(!stop.contains(Signal::Hangup));
            assert!(notify.contains(Signal::Hangup));
            assert!(notify.contains(Signal::User1));
        } else {
            assert!(stop.contains(Signal::Hangup));
        }
    }

    #[test]
    fn test_handle_stop_signal_requests_stop_once() {
        let kernel = Kernel::new();
        let (stop, _) = signal_sets();
        let actions = Actions::default();
        handle(
            Signal::Terminate,
            stop,
            &kernel,
            &LogSlot::default(),
            &actions,
        );
        handle(
            Signal::Interrupt,
            stop,
            &kernel,
            &LogSlot::default(),
            &actions,
        );
        assert_eq!(
            kernel.stop_reason(),
            Some(StopReason::Signal {
                name: signal_name(Signal::Terminate)
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_handle_hangup_and_user1_do_not_stop_on_unix() {
        let kernel = Kernel::new();
        let (stop, _) = signal_sets();
        let dumped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&dumped);
        let reloaded = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reload_flag = Arc::clone(&reloaded);
        let actions = Actions {
            reload_on: true,
            dump_on: true,
            ..Actions::default()
        };
        drop(actions.dump.set(Box::new(move || {
            flag.store(true, std::sync::atomic::Ordering::SeqCst)
        })));
        drop(actions.reload.set(Box::new(move || {
            reload_flag.store(true, std::sync::atomic::Ordering::SeqCst)
        })));
        handle(Signal::Hangup, stop, &kernel, &LogSlot::default(), &actions);
        handle(Signal::User1, stop, &kernel, &LogSlot::default(), &actions);
        assert_eq!(kernel.stop_reason(), None);
        assert!(dumped.load(std::sync::atomic::Ordering::SeqCst));
        assert!(reloaded.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn test_intake_installs_and_uninstalls() {
        let kernel = Kernel::new();
        let intake = SignalIntake::install(kernel.clone(), LogSlot::default(), true, true);
        assert!(intake.is_ok());
        if let Ok(intake) = intake
            && let Some(thread) = intake.uninstall()
        {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let outcome = crate::shutdown::run_with_deadline("join", deadline, move || {
                drop(thread.join());
            });
            assert!(matches!(outcome, crate::shutdown::Outcome::Done(_)));
        }
        assert_eq!(kernel.stop_reason(), None);
    }
}
