//! Running installed packages' background work: scheduled tasks (see
//! `scheduled`).
//!
//! Background work runs only in a launcher made to run it
//! ([`Launcher::with_background`]): a thread of its own (the driver) looks
//! at what is due whenever something changed (the user turned a schedule on
//! or off, a package was enabled, reloaded, retried or otherwise changed, a
//! run ended) and whenever the next piece of work falls due on the
//! launcher's clock, then starts it in the runtime and waits for it there.
//! The runtime runs it beside the calls it serves (see
//! [`crate::runtime::Runtime::run_task_with`]); the window's thread never
//! waits for it.
//!
//! The work of a package belongs to its generation, like its calls: it runs
//! only while the package is enabled and not paused, and never while
//! something else changes the package (a reload, an update, an uninstall).
//! A package's generation ending stops its work where it waits.

use std::future::Future;
use std::sync::Arc;
use std::task::Poll;
use std::time::SystemTime;

use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

use super::{Launcher, WeakLauncher};
use crate::clock::Clock;

/// What a launcher that runs background work shares with its driver.
pub(super) struct Background {
    /// The clock background work is timed by.
    pub(super) clock: Arc<dyn Clock>,
    /// Wakes the driver to look at what is due again. Dropping it (with the
    /// last launcher) stops the driver.
    wake: mpsc::UnboundedSender<()>,
    /// Counts the changes of the background work's state, for those waiting
    /// on them ([`Launcher::background_changes`]).
    revision: watch::Sender<u64>,
}

impl Background {
    /// Has the driver look at what is due again.
    pub(super) fn wake(&self) {
        let _ = self.wake.send(());
    }

    /// Tells those waiting for the background work that it changed.
    pub(super) fn changed(&self) {
        self.revision.send_modify(|revision| *revision += 1);
    }
}

/// A piece of background work the driver waits for, and what it answers
/// once it ends.
pub(super) type Pending = std::pin::Pin<Box<dyn Future<Output = Finished> + Send>>;

/// Background work that ended, for the launcher to note.
pub(super) enum Finished {
    Task(super::scheduled::Ended),
}

/// Changes of the state of a launcher's background work: its tasks' runs
/// starting and ending. Made by [`Launcher::background_changes`].
pub struct BackgroundChanges(watch::Receiver<u64>);

impl BackgroundChanges {
    /// Resolves once the background work changed since this was made, or
    /// since the last call; `None` once the launcher has stopped. Several
    /// changes meanwhile are one.
    pub async fn next(&mut self) -> Option<()> {
        self.0.changed().await.ok()
    }
}

impl Launcher {
    /// This launcher running installed packages' background work (their
    /// scheduled tasks), timed by `clock`, normally
    /// [`crate::clock::SystemClock`]; a thread of its own starts what is
    /// due, from now on, until every handle to this launcher is dropped.
    /// Without it, turning a schedule on explains that this Pane runs no
    /// background work. Tell the window about changes first
    /// ([`Launcher::with_development`]): the background work tells it
    /// through the launcher this makes.
    pub fn with_background(self, clock: Arc<dyn Clock>) -> Self {
        let (wake, woken) = mpsc::unbounded_channel();
        let background = Arc::new(Background {
            clock: clock.clone(),
            wake,
            revision: watch::Sender::new(0),
        });
        self.lock().background = Some(Arc::downgrade(&background));
        let launcher = Launcher {
            background: Some(background),
            ..self
        };
        // The runtime reports failures to this launcher too.
        launcher.report_failures();
        let driver = launcher.downgrade();
        std::thread::Builder::new()
            .name("pane-background".into())
            .spawn(move || drive(driver, clock, woken))
            .expect("the background work's thread could not start");
        launcher
    }

    /// Changes of the state of this launcher's background work, from now
    /// on: its tasks' runs starting and ending.
    pub fn background_changes(&self) -> BackgroundChanges {
        let revision = match &self.background {
            Some(background) => background.revision.subscribe(),
            // Never changes.
            None => watch::Sender::new(0).subscribe(),
        };
        BackgroundChanges(revision)
    }

    /// Has the driver look at what is due again, and tells those waiting
    /// that the background work changed.
    pub(super) fn background_changed(&self) {
        if let Some(background) = &self.background {
            background.wake();
            background.changed();
        }
    }

    /// The time on the background work's clock.
    pub(super) fn now(&self) -> SystemTime {
        match &self.background {
            Some(background) => background.clock.now(),
            None => SystemTime::now(),
        }
    }

    /// Starts the background work that is due, returning what to wait for
    /// and when the next piece falls due, if one is waiting.
    fn start_due(&self) -> (Vec<Pending>, Option<SystemTime>) {
        let now = self.now();
        let (started, next, save) = {
            let mut state = self.lock();
            self.start_due_tasks(&mut state, now)
        };
        if save {
            // Recorded before anyone is told, so what they read is on record.
            self.record_tasks();
            self.changed();
            if let Some(background) = &self.background {
                background.changed();
            }
        }
        (started, next)
    }

    /// Notes that background work ended.
    fn finish(&self, finished: Finished) {
        match finished {
            Finished::Task(ended) => self.finish_task(ended),
        }
    }
}

/// What woke the driver.
enum Woken {
    Finished(Finished),
    Looked,
    Stopped,
}

/// The driver: starts what is due, then waits for a run to end, the next
/// piece of work to fall due or a change, until the launcher is gone.
fn drive(launcher: WeakLauncher, clock: Arc<dyn Clock>, mut woken: mpsc::UnboundedReceiver<()>) {
    let executor = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(executor) => executor,
        Err(error) => {
            eprintln!("pane: background work cannot run: {error}");
            return;
        }
    };
    executor.block_on(async move {
        let mut runs: JoinSet<Finished> = JoinSet::new();
        loop {
            let next = {
                let Some(launcher) = launcher.upgrade() else {
                    return;
                };
                let (started, next) = launcher.start_due();
                for run in started {
                    runs.spawn(run);
                }
                next
            };
            let mut due = next.map(|at| clock.reached(at));
            let woke = std::future::poll_fn(|cx| {
                if let Poll::Ready(Some(Ok(finished))) = runs.poll_join_next(cx) {
                    return Poll::Ready(Woken::Finished(finished));
                }
                match woken.poll_recv(cx) {
                    Poll::Ready(Some(())) => {
                        // Several changes meanwhile are one look.
                        while woken.try_recv().is_ok() {}
                        return Poll::Ready(Woken::Looked);
                    }
                    Poll::Ready(None) => return Poll::Ready(Woken::Stopped),
                    Poll::Pending => {}
                }
                if let Some(due) = &mut due
                    && due.as_mut().poll(cx).is_ready()
                {
                    return Poll::Ready(Woken::Looked);
                }
                Poll::Pending
            })
            .await;
            match woke {
                Woken::Finished(finished) => {
                    let Some(launcher) = launcher.upgrade() else {
                        return;
                    };
                    launcher.finish(finished);
                }
                Woken::Looked => {}
                Woken::Stopped => return,
            }
        }
    });
}
