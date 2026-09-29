//! Keeping Pane usable when its extension runtime thread crashes (#17).
//!
//! The runtime thread serves every guest call. A panic on it (a fault in
//! Pane's host code or in Wasmtime, not a guest trap, which is a crash of
//! one package) unwinds it: its instances, views and pending calls are
//! dropped, so every call that was running or queued answers
//! [`CallError::RuntimeUnavailable`] and none is sent again. The thread
//! catches its own unwinding, ends the native helpers still running (they
//! were all started by its guests), and then:
//!
//! - starts a fresh runtime thread, so the next call the user asks for runs
//!   there; or,
//! - when it crashed within [`CRASH_WINDOW`] of the previous crash, starts
//!   none: repeated automatic restarts are suppressed, and the runtime stays
//!   stopped until the user restarts it ([`Runtime::restart`]).
//!
//! Either way the launcher is told ([`CrashReport`]), without naming any
//! package: which one, if any, caused a crash of the shared thread is not
//! known, so nothing is paused. Extension data is untouched.
//!
//! This needs the panic to unwind (checked at compile time below). A panic
//! in a destructor while the thread unwinds from a first panic aborts the
//! whole process, as does any other abort: neither is recovered.

use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};

#[cfg(any(test, debug_assertions))]
use super::faults::Fault;
use super::faults::Faults;

use super::{
    CallError, Code, HealthReport, Host, Request, SharedApplications, SharedClipboard,
    SharedDirectory, lock, unavailable,
};
use crate::helpers::runner::Helpers;

// Recovering from a crash of the runtime thread relies on its panic
// unwinding to a `catch_unwind` on that thread: with `panic = "abort"`, any
// panic there ends Pane's whole process.
#[cfg(not(panic = "unwind"))]
compile_error!(
    "Pane must be built with `panic = \"unwind\"`: it recovers from a crash of its extension runtime thread by catching its panic"
);

/// How close together crashes count together: three crashes of a package
/// within it pause the package (see the launcher's `pausing`), and a second
/// crash of the runtime thread within it after the previous one stops the
/// runtime instead of restarting it. An explicit choice (provisional).
pub(crate) const CRASH_WINDOW: Duration = Duration::from_secs(5 * 60);

/// What the runtime is doing, as far as crashes of its thread go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeStatus {
    /// It runs, and has not crashed since it started or was last restarted
    /// by the user.
    Running,
    /// It crashed, and Pane started it again by itself. `why` is what its
    /// thread reported as it stopped.
    Restarted { why: String },
    /// It crashed and was not started again: it runs nothing until the user
    /// restarts it. `not_restarted` says why Pane did not.
    Stopped { why: String, not_restarted: String },
}

/// Told on the crashed runtime thread, once its helpers were ended and it
/// was restarted or not, with the crashed thread's number (see
/// [`super::ViewId::thread`]) and what the runtime does now.
pub(crate) type CrashReport = Arc<dyn Fn(u64, &RuntimeStatus) + Send + Sync>;

/// What every [`super::Runtime`] handle shares: the runtime thread now
/// serving calls, if one is, and what a new one needs.
pub(super) struct Shared {
    /// The native helper processes guests started, which end once every
    /// handle is dropped.
    pub(super) helpers: Helpers,
    pub(super) applications: SharedApplications,
    pub(super) files: crate::files::FileAccess,
    pub(super) clipboard: SharedClipboard,
    pub(super) directory: SharedDirectory,
    pub(super) health: Arc<Mutex<Option<HealthReport>>>,
    /// Custom view ids, never reused, even by a restarted thread: a view
    /// the window still shows from a crashed one must not name a new view.
    pub(super) next_view: Arc<AtomicU64>,
    /// Guests' web requests: their limits, and what each package did this
    /// session, which a restarted thread carries on.
    pub(super) network: Arc<crate::http::Network>,
    crashes: Mutex<Option<CrashReport>>,
    /// Counts the crashed threads Pane is done with (restarted or not, the
    /// launcher told), for a call whose answer a crash lost.
    handled: watch::Sender<u64>,
    cache_dir: Option<PathBuf>,
    current: Mutex<Current>,
}

/// Why a request was not sent to the runtime thread.
pub(super) enum NotSent {
    /// The runtime is stopped after crashing.
    Stopped,
    /// The thread has just crashed; Pane is handling it.
    Lost,
}

impl NotSent {
    /// How a request that answers nothing says it was not sent.
    pub(super) fn error(self) -> CallError {
        match self {
            NotSent::Stopped => stopped(),
            NotSent::Lost => lost_in(&RuntimeStatus::Running),
        }
    }
}

/// Counts a crashed thread as handled when it ends, however its handling
/// ends.
struct Handled(Weak<Shared>);

impl Drop for Handled {
    fn drop(&mut self) {
        if let Some(shared) = self.0.upgrade() {
            shared.handled.send_modify(|handled| *handled += 1);
        }
    }
}

impl Drop for Shared {
    /// The last handle is gone (Pane is quitting): so are the helpers,
    /// which would otherwise outlive it. The thread stops with its requests.
    fn drop(&mut self) {
        self.helpers.stop_all();
    }
}

struct Current {
    /// The thread serving calls; `None` while the runtime is stopped.
    thread: Option<Thread>,
    /// Counts the threads started, to tell a report of an old one.
    started: u64,
    /// When the runtime last crashed, since the user last restarted it.
    last_crash: Option<Instant>,
    status: RuntimeStatus,
}

/// A runtime thread's end of its requests.
#[derive(Clone)]
struct Thread {
    requests: mpsc::UnboundedSender<Request>,
    /// Where faults are injected into it.
    #[cfg(any(test, debug_assertions))]
    faults: Arc<Faults>,
}

impl Shared {
    /// Starts the first runtime thread with `code`.
    pub(super) fn start(
        code: Arc<Code>,
        applications: SharedApplications,
        helpers: Helpers,
        cache_dir: Option<PathBuf>,
    ) -> Result<Arc<Shared>, CallError> {
        let shared = Arc::new(Shared {
            helpers,
            applications,
            files: crate::files::FileAccess::default(),
            clipboard: SharedClipboard::default(),
            directory: SharedDirectory::default(),
            health: Arc::default(),
            next_view: Arc::default(),
            network: Arc::default(),
            crashes: Mutex::new(None),
            handled: watch::Sender::new(0),
            cache_dir,
            current: Mutex::new(Current {
                thread: None,
                started: 0,
                last_crash: None,
                status: RuntimeStatus::Running,
            }),
        });
        {
            let mut current = lock(&shared.current);
            let thread = shared.spawn(code, 1)?;
            current.thread = Some(thread);
            current.started = 1;
        }
        Ok(shared)
    }

    /// Sends `request` to the runtime thread.
    pub(super) fn send(&self, request: Request) -> Result<(), NotSent> {
        let requests = match &lock(&self.current).thread {
            Some(thread) => thread.requests.clone(),
            None => return Err(NotSent::Stopped),
        };
        // A thread that just crashed has dropped its requests: this one is
        // not sent, nor sent again to the next thread.
        requests.send(request).map_err(|_| NotSent::Lost)
    }

    /// Resolves, through [`lost`], once a thread that crashes after this
    /// call has been handled.
    pub(super) fn handled(&self) -> watch::Receiver<u64> {
        self.handled.subscribe()
    }

    pub(super) fn status(&self) -> RuntimeStatus {
        lock(&self.current).status.clone()
    }

    pub(super) fn set_crash_report(&self, report: CrashReport) {
        *lock(&self.crashes) = Some(report);
    }

    #[cfg(any(test, debug_assertions))]
    pub(super) fn inject(&self, fault: Fault) {
        let Some(thread) = lock(&self.current).thread.clone() else {
            return;
        };
        thread.faults.inject(fault);
    }

    /// Starts the runtime again after it stopped, at the user's request:
    /// crashes before this no longer count against restarting it by
    /// itself. Nothing that ran before is run again. A runtime that runs is
    /// left as it is.
    pub(super) fn restart(self: &Arc<Shared>) -> Result<(), CallError> {
        let mut current = lock(&self.current);
        if current.thread.is_some() {
            return Ok(());
        }
        if self.helpers.quitting() {
            return Err(CallError::RuntimeUnavailable("Pane is quitting".into()));
        }
        let number = current.started + 1;
        let thread = self.spawn(Arc::new(Code::new(self.engine()?)), number)?;
        current.thread = Some(thread);
        current.started = number;
        current.last_crash = None;
        current.status = RuntimeStatus::Running;
        Ok(())
    }

    fn engine(&self) -> Result<wasmtime::Engine, CallError> {
        super::engine(self.cache_dir.clone())
    }

    /// Starts runtime thread number `number`, serving calls with `code`.
    fn spawn(self: &Arc<Shared>, code: Arc<Code>, number: u64) -> Result<Thread, CallError> {
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(unavailable)?;
        let (requests, receiver) = mpsc::unbounded_channel();
        let faults = Arc::new(Faults::default());
        let host = Host::new(code, self, number, Arc::clone(&faults));
        let shared = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("pane-extension-runtime".into())
            .spawn(move || {
                // Dropped last, once the crash (if any) was handled.
                let _handled = Handled(shared.clone());
                let served = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    executor.block_on(host.serve(receiver))
                }));
                drop(executor);
                if let Err(panic) = served {
                    crashed(&shared, number, panic_message(&*panic));
                }
            })
            .map_err(unavailable)?;
        Ok(Thread {
            requests,
            #[cfg(any(test, debug_assertions))]
            faults,
        })
    }
}

/// Runtime thread `number` crashed with `why`, and its unwinding dropped
/// everything it held: ends the helpers its guests left running, restarts
/// it unless it crashed within [`CRASH_WINDOW`] of its previous crash,
/// and tells the launcher.
fn crashed(shared: &Weak<Shared>, number: u64, why: String) {
    let Some(shared) = shared.upgrade() else {
        // Pane is quitting.
        return;
    };
    // Every helper still running was started by the crashed thread's
    // guests: no other thread runs until this one has been handled.
    shared.helpers.stop_running();
    let status = {
        let mut current = lock(&shared.current);
        // Only the thread serving calls runs guests and so can crash: a
        // restart replaces a thread only once it has stopped.
        current.thread = None;
        let now = Instant::now();
        let again = restarts_automatically(current.last_crash, now);
        current.last_crash = Some(now);
        let restarted = if shared.helpers.quitting() {
            Err("Pane is quitting".to_owned())
        } else if !again {
            Err(format!(
                "it crashed twice within {} minutes; repeated automatic restarts are suppressed",
                CRASH_WINDOW.as_secs() / 60
            ))
        } else {
            let next = number + 1;
            shared
                .engine()
                .and_then(|engine| shared.spawn(Arc::new(Code::new(engine)), next))
                .map(|thread| {
                    current.thread = Some(thread);
                    current.started = next;
                })
                .map_err(|error| format!("starting it again failed: {error}"))
        };
        current.status = match restarted {
            Ok(()) => RuntimeStatus::Restarted { why },
            Err(not_restarted) => RuntimeStatus::Stopped { why, not_restarted },
        };
        current.status.clone()
    };
    eprintln!("Pane's extension runtime stopped unexpectedly: {status:?}");
    let report = lock(&shared.crashes).clone();
    if let Some(report) = report {
        report(number, &status);
    }
}

/// Whether a crash at `now` restarts the runtime by itself: not when it
/// already crashed within [`CRASH_WINDOW`] before (`previous`, since the
/// user last restarted it).
pub(crate) fn restarts_automatically(previous: Option<Instant>, now: Instant) -> bool {
    previous.is_none_or(|previous| now.saturating_duration_since(previous) >= CRASH_WINDOW)
}

/// The message of a panic, for diagnostics.
fn panic_message(panic: &(dyn Any + Send)) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "the runtime thread panicked".to_owned()
    }
}

/// How a call answers when the runtime is stopped: it is not sent.
pub(super) fn stopped() -> CallError {
    CallError::RuntimeUnavailable(
        "it stopped after crashing and runs nothing until you restart it in Manage extensions"
            .into(),
    )
}

/// How a call answers when the runtime thread crashed before answering
/// it, once Pane has handled the crash (`handled` changed): whether it
/// restarted the runtime. It is not sent again.
pub(super) async fn lost(shared: Weak<Shared>, mut handled: watch::Receiver<u64>) -> CallError {
    // The thread counts itself as handled however its handling ends; the
    // count's sender goes only with the last runtime handle.
    let _ = handled.changed().await;
    let status = match shared.upgrade() {
        Some(shared) => shared.status(),
        None => RuntimeStatus::Running,
    };
    lost_in(&status)
}

/// How a call whose answer was lost answers, while the runtime does
/// `status`.
fn lost_in(status: &RuntimeStatus) -> CallError {
    CallError::RuntimeUnavailable(match status {
        RuntimeStatus::Restarted { .. } => "it stopped before answering and was started again; \
                                            Pane does not run this again by itself"
            .into(),
        RuntimeStatus::Stopped { not_restarted, .. } => format!(
            "it stopped before answering and was not restarted ({not_restarted}); Pane does not \
             run this again by itself. Restart it in Manage extensions"
        ),
        RuntimeStatus::Running => {
            "it stopped before answering; Pane does not run this again by itself".into()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_crash_restarts_the_runtime() {
        assert!(restarts_automatically(None, Instant::now()));
    }

    #[test]
    fn a_second_crash_within_the_window_does_not() {
        let first = Instant::now();
        let soon = first + CRASH_WINDOW - Duration::from_secs(1);
        assert!(!restarts_automatically(Some(first), soon));
    }

    #[test]
    fn a_crash_after_the_window_restarts_it_again() {
        let first = Instant::now();
        assert!(restarts_automatically(Some(first), first + CRASH_WINDOW));
    }

    #[test]
    fn a_panic_message_is_kept() {
        let panic = std::panic::catch_unwind(|| panic!("broke {}", 1)).unwrap_err();
        assert_eq!(panic_message(&*panic), "broke 1");
        let panic = std::panic::catch_unwind(|| panic!("static")).unwrap_err();
        assert_eq!(panic_message(&*panic), "static");
    }
}
