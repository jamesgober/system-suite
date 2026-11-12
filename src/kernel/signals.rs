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
    /// Unix: SIGHUP stops instead of reloading (see [`hangup_stops`]).
    hangup_stops: bool,
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
    /// `[kernel] dump_signal` off SIGUSR1 is still received (its default
    /// action would end the process) and ignored. SIGHUP stops the process
    /// instead of reloading when `[kernel] reload_signal` is off, or when
    /// the process runs at a terminal with no service manager (the
    /// terminal went away; see [`hangup_stops`]).
    pub(crate) fn install(
        kernel: Kernel,
        log: LogSlot,
        reload_on: bool,
        dump_on: bool,
    ) -> Result<SignalIntake, Error> {
        let (stop, notify) = signal_sets();
        let coordinator = coordinator(stop, notify);
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
            hangup_stops: hangup_stops(reload_on, at_terminal(), under_service_manager()),
            ..Actions::default()
        });
        let thread_actions = Arc::clone(&actions);
        let spawned = thread::Builder::new()
            .name("system-suite-signals".to_owned())
            .spawn(move || {
                while let Some(signal) = subscription.recv_blocking() {
                    // A panic in a handler (application code reached from a
                    // dump or a reload) must not end the intake: stop
                    // signals would then be ignored.
                    let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        handle(signal, stop, &kernel, &log, &thread_actions);
                    }));
                    if handled.is_err() {
                        log.error("a signal handler panicked; the intake goes on", &[]);
                    }
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
    let hangup_stop = cfg!(unix) && signal == Signal::Hangup && actions.hangup_stops;
    if stop.contains(signal) || hangup_stop {
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
        s if s == RELOAD && !actions.reload_on => log.info(
            "reload signal ignored: [kernel] reload_signal is off",
            &[("signal", Val::Str(name))],
        ),
        s if s == RELOAD => match actions.reload.get() {
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

/// The signal that asks for a reload: SIGHUP on Unix; on Windows the
/// service manager's parameter-change control, which signal-mod delivers
/// as `User2`.
#[cfg(unix)]
const RELOAD: Signal = Signal::Hangup;
#[cfg(not(unix))]
const RELOAD: Signal = Signal::User2;

/// Whether SIGHUP stops the process instead of reloading it (Unix).
///
/// Under a service manager SIGHUP means "reload". At a terminal it means
/// the terminal went away: reloading would leave a detached process that
/// still holds the single-instance lock. With `reload_signal` off nothing
/// else would act on it, so it keeps its default meaning, stop.
fn hangup_stops(reload_on: bool, at_terminal: bool, service_manager: bool) -> bool {
    !reload_on || (at_terminal && !service_manager)
}

/// Standard input or standard error is a terminal.
fn at_terminal() -> bool {
    use std::io::IsTerminal as _;
    std::io::stdin().is_terminal() || std::io::stderr().is_terminal()
}

/// A service manager started the process: systemd sets `INVOCATION_ID`
/// for every unit and `NOTIFY_SOCKET` for `Type=notify`, launchd sets
/// `XPC_SERVICE_NAME`.
fn under_service_manager() -> bool {
    ["INVOCATION_ID", "NOTIFY_SOCKET", "XPC_SERVICE_NAME"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty() && v != "0"))
}

/// The stop and notify sets for this OS (D-102).
///
/// The stop set is signal-mod's [`SignalSet::stop`]: SIGTERM and SIGINT
/// (on Windows console close and Ctrl+C; the service manager's stop and
/// shutdown arrive as `Terminate`). SIGQUIT keeps the meaning the OS gives
/// it, quit with a core dump, for a process that hangs. On Windows,
/// Ctrl+Break (`Quit`) stops too, since its default there ends the process
/// without a shutdown. SIGHUP is the reload signal on Unix (see
/// [`hangup_stops`]); signal-mod 2.2 no longer reports a Windows logoff at
/// all, so a user logging off never reaches the kernel.
#[cfg(unix)]
fn signal_sets() -> (SignalSet, SignalSet) {
    let notify = SignalSet::empty().with(Signal::Hangup).with(Signal::User1);
    (SignalSet::stop(), notify)
}

#[cfg(windows)]
fn signal_sets() -> (SignalSet, SignalSet) {
    // Only the service control manager produces User1 and User2 on
    // Windows: user control code 128 (dump) and parameter change (reload).
    let notify = SignalSet::empty().with(Signal::User1).with(Signal::User2);
    (SignalSet::stop().with(Signal::Quit), notify)
}

#[cfg(not(any(unix, windows)))]
fn signal_sets() -> (SignalSet, SignalSet) {
    (SignalSet::empty(), SignalSet::empty())
}

/// The coordinator for `stop` and `notify`.
///
/// Unix: signal-mod's default builder with the notify set, whose stop set
/// is then [`SignalSet::stop`] (its default minus SIGHUP), and which
/// leaves a stop signal ignored before boot ignored: SIGHUP under `nohup`,
/// or SIGINT for a job a script started in the background. An explicit
/// set would catch those too.
#[cfg(unix)]
fn coordinator(stop: SignalSet, notify: SignalSet) -> Coordinator {
    debug_assert!(stop == SignalSet::stop());
    Coordinator::builder().notify_signals(notify).build()
}

/// Elsewhere the stop set is given as it is: nothing is ignored before
/// boot on Windows, and Ctrl+Break is added to the default.
#[cfg(not(unix))]
fn coordinator(stop: SignalSet, notify: SignalSet) -> Coordinator {
    Coordinator::builder()
        .signals(stop)
        .notify_signals(notify)
        .build()
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
        Signal::User1 => "SERVICE_CONTROL_128",
        Signal::User2 => "SERVICE_CONTROL_PARAMCHANGE",
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
        assert!(!stop.contains(Signal::Hangup));
        if cfg!(unix) {
            // D-102: signal-mod's stop set; SIGQUIT keeps its core dump.
            assert_eq!(stop, SignalSet::stop());
            assert!(!stop.contains(Signal::Quit));
            assert!(notify.contains(Signal::Hangup));
            assert!(notify.contains(Signal::User1));
        } else {
            // Ctrl+Break stops gracefully; a logoff is never delivered.
            assert!(stop.contains(Signal::Quit));
            assert!(!notify.contains(Signal::Hangup));
        }
    }

    #[test]
    fn test_hangup_stops_only_at_a_terminal_or_with_reload_off() {
        // reload_on, at_terminal, service_manager
        assert!(!hangup_stops(true, false, false));
        assert!(!hangup_stops(true, false, true));
        assert!(!hangup_stops(true, true, true));
        assert!(hangup_stops(true, true, false));
        assert!(hangup_stops(false, false, true));
        assert!(hangup_stops(false, true, false));
    }

    #[test]
    fn test_handle_hangup_stops_when_marked() {
        let kernel = Kernel::new();
        let (stop, _) = signal_sets();
        let actions = Actions {
            reload_on: true,
            hangup_stops: true,
            ..Actions::default()
        };
        handle(Signal::Hangup, stop, &kernel, &LogSlot::default(), &actions);
        if cfg!(unix) {
            assert_eq!(
                kernel.stop_reason(),
                Some(StopReason::Signal { name: "SIGHUP" })
            );
        } else {
            // Windows: Hangup is never delivered (signal-mod 2.2), and the
            // flag is for Unix only.
            assert_eq!(kernel.stop_reason(), None);
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
