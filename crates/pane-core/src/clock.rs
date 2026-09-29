//! The clock Pane times background work by: when a scheduled task is due
//! (#47), and when a service that crashed is started again (#48).
//!
//! Schedules are kept in wall-clock time ([`SystemTime`]), since a task's
//! last run is recorded and must still say when it ran after Pane restarts.
//! The launcher asks a [`Clock`] rather than the system directly, so the
//! tests drive schedules with a [`ManualClock`] instead of waiting for them.

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime};

/// What resolves once a clock reaches a time.
pub type Reached = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A clock background work is timed by.
pub trait Clock: Send + Sync {
    /// The time now.
    fn now(&self) -> SystemTime;

    /// Resolves once it is `at` or later on this clock. Awaited on a thread
    /// of Pane's own that runs a Tokio runtime with its timer enabled.
    fn reached(&self, at: SystemTime) -> Reached;
}

/// The system's clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

/// How long [`SystemClock`] sleeps at most before it looks at the time
/// again, so that a change of the system's time (or its sleep) delays a
/// schedule by no more than this.
const LOOK_AGAIN: Duration = Duration::from_secs(30);

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }

    fn reached(&self, at: SystemTime) -> Reached {
        Box::pin(async move {
            while let Ok(left) = at.duration_since(SystemTime::now()) {
                if left.is_zero() {
                    break;
                }
                tokio::time::sleep(left.min(LOOK_AGAIN)).await;
            }
        })
    }
}

/// A clock that stands still until it is moved on, for tests: nothing
/// timed by it waits for real time to pass. Cloning shares the same time.
/// Debug builds only.
#[cfg(any(test, debug_assertions))]
#[derive(Clone, Debug)]
pub struct ManualClock(std::sync::Arc<tokio::sync::watch::Sender<SystemTime>>);

#[cfg(any(test, debug_assertions))]
impl ManualClock {
    /// A clock showing `now`.
    pub fn new(now: SystemTime) -> ManualClock {
        ManualClock(std::sync::Arc::new(tokio::sync::watch::Sender::new(now)))
    }

    /// Moves the clock on by `by`, reaching what was waiting for it.
    pub fn advance(&self, by: Duration) {
        self.0.send_modify(|now| *now += by);
    }

    /// Sets the clock to `at`, which may be earlier.
    pub fn set(&self, at: SystemTime) {
        self.0.send_replace(at);
    }
}

#[cfg(any(test, debug_assertions))]
impl Clock for ManualClock {
    fn now(&self) -> SystemTime {
        *self.0.borrow()
    }

    fn reached(&self, at: SystemTime) -> Reached {
        let mut now = self.0.subscribe();
        Box::pin(async move {
            // The sender lives as long as the clock's clones; one gone, the
            // time never comes.
            if now.wait_for(|now| *now >= at).await.is_err() {
                std::future::pending::<()>().await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;

    #[test]
    fn a_manual_clock_reaches_a_time_only_once_moved_there() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let clock = ManualClock::new(start);
        let mut reached = clock.reached(start + Duration::from_secs(60));
        assert!((&mut reached).now_or_never().is_none());
        clock.advance(Duration::from_secs(59));
        assert!((&mut reached).now_or_never().is_none());
        clock.advance(Duration::from_secs(1));
        assert_eq!(reached.now_or_never(), Some(()));
        assert_eq!(clock.now(), start + Duration::from_secs(60));
        assert_eq!(clock.reached(start).now_or_never(), Some(()));
    }
}
