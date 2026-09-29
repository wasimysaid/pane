//! Threads of Pane's own that a caller can stop without waiting forever:
//! [`Joinable`] is joined within a time limit, or else left to finish on
//! its own. On Windows, [`windows::MessageThread`] is such a thread with a
//! message queue, which the clipboard listener and the hotkey adapter use.
//! Only Windows uses [`Joinable`] so far; its tests run everywhere.
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use std::sync::{Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

#[cfg(target_os = "windows")]
pub(crate) mod windows;

/// A thread that tells when it ends, however it ends.
pub(crate) struct Joinable {
    handle: JoinHandle<()>,
    /// Disconnected once the thread ended: its sender is dropped then, on
    /// return and on a panic alike. In a mutex only so that its owner can
    /// be shared between threads.
    ended: Mutex<mpsc::Receiver<()>>,
}

impl Joinable {
    /// Starts `run` on a new thread called `name`.
    pub(crate) fn spawn(
        name: &str,
        run: impl FnOnce() + Send + 'static,
    ) -> std::io::Result<Joinable> {
        let (ending, ended) = mpsc::channel::<()>();
        let handle = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let _ending = ending;
                run();
            })?;
        Ok(Joinable {
            handle,
            ended: Mutex::new(ended),
        })
    }

    /// Waits at most `limit` for the thread to end, and joins it if it
    /// did; otherwise leaves it running, detached. Returns whether it
    /// ended.
    pub(crate) fn join_within(self, limit: Duration) -> bool {
        let ended = self
            .ended
            .into_inner()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match ended.recv_timeout(limit) {
            Err(mpsc::RecvTimeoutError::Timeout) => false,
            // Disconnected: the thread is ending, and joining it is at most
            // a moment's wait. It never sends.
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = self.handle.join();
                true
            }
        }
    }

    /// Waits for the thread to end, however long it takes.
    pub(crate) fn join(self) {
        let _ = self.handle.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    #[test]
    fn a_thread_that_ends_in_time_is_joined() {
        let done = Arc::new(AtomicBool::new(false));
        let set = done.clone();
        let thread =
            Joinable::spawn("test-quick", move || set.store(true, Ordering::SeqCst)).unwrap();
        assert!(thread.join_within(Duration::from_secs(5)));
        assert!(done.load(Ordering::SeqCst));
    }

    #[test]
    fn a_thread_that_panics_counts_as_ended() {
        let thread = Joinable::spawn("test-panics", || panic!("on purpose")).unwrap();
        assert!(thread.join_within(Duration::from_secs(5)));
    }

    #[test]
    fn a_hung_thread_is_left_running_once_the_limit_passes() {
        let (release, released) = mpsc::channel::<()>();
        let thread = Joinable::spawn("test-hung", move || {
            let _ = released.recv();
        })
        .unwrap();
        let started = Instant::now();
        assert!(!thread.join_within(Duration::from_millis(100)));
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(100), "{waited:?}");
        assert!(waited < Duration::from_secs(2), "{waited:?}");
        // The detached thread ends once released.
        let _ = release.send(());
    }
}
