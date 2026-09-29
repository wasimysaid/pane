//! A thread of Pane's own with a Windows message queue, for adapters that
//! need one: the clipboard listener (whose message-only window receives
//! `WM_CLIPBOARDUPDATE`) and the hotkey adapter (whose thread receives
//! `WM_HOTKEY`). The thread messages Pane posts to such threads are defined
//! here, once.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

use ::windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use ::windows::Win32::System::LibraryLoader::GetModuleHandleW;
use ::windows::Win32::System::Threading::GetCurrentThreadId;
use ::windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG, PM_NOREMOVE,
    PeekMessageW, PostQuitMessage, PostThreadMessageW, RegisterClassW, SendNotifyMessageW,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_USER, WNDCLASSW,
};
use ::windows::core::PCWSTR;

use super::Joinable;

/// Wakes a thread to serve what was queued for it.
pub(crate) const WM_WAKE: u32 = WM_APP + 1;
/// Ends a thread's message loop.
pub(crate) const WM_STOP: u32 = WM_APP + 2;

/// How often, and how long apart, Pane tries to post a message to a thread
/// whose queue is full.
const POST_TRIES: u32 = 5;
const POST_WAIT: Duration = Duration::from_millis(10);

/// A window procedure.
pub(crate) type Procedure = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

/// A window class of Pane's, registered when first used.
pub(crate) struct WindowClass {
    name: &'static str,
    procedure: Procedure,
    /// The class's name as a NUL-terminated wide string once registered,
    /// or why it could not be.
    registered: OnceLock<Result<Vec<u16>, String>>,
}

impl WindowClass {
    pub(crate) const fn new(name: &'static str, procedure: Procedure) -> WindowClass {
        WindowClass {
            name,
            procedure,
            registered: OnceLock::new(),
        }
    }

    fn register(&self) -> Result<PCWSTR, String> {
        let registered = self.registered.get_or_init(|| {
            let mut name: Vec<u16> = self.name.encode_utf16().collect();
            name.push(0);
            // SAFETY: no arguments; the module is this process's executable.
            let instance =
                unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|error| error.message())?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(self.procedure),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(name.as_ptr()),
                ..WNDCLASSW::default()
            };
            // SAFETY: `class` is fully initialized; its name is kept, with
            // this class, for as long as the process runs.
            if unsafe { RegisterClassW(&class) } == 0 {
                return Err(::windows::core::Error::from_thread().message());
            }
            Ok(name)
        });
        match registered {
            Ok(name) => Ok(PCWSTR(name.as_ptr())),
            Err(problem) => Err(problem.clone()),
        }
    }

    /// A message-only window of this class, owned by the calling thread.
    pub(crate) fn message_window(&'static self) -> Result<Window, String> {
        let class = self.register()?;
        // SAFETY: no arguments; the module is this process's executable.
        let instance =
            unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|error| error.message())?;
        // SAFETY: the class is registered and its name kept; a message-only
        // window has no title, size, menu or creation data.
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                PCWSTR::null(),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance.into()),
                None,
            )
        }
        .map_err(|error| error.message())?;
        Ok(Window(window))
    }
}

/// A window of the calling thread, destroyed when dropped (by that thread).
pub(crate) struct Window(HWND);

impl Window {
    pub(crate) fn handle(&self) -> HWND {
        self.0
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: this thread's own window.
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

/// For the window procedure of a thread's window: ends the thread's message
/// loop if `message` is [`WM_STOP`], which is sent to the window when it
/// could not be posted to the thread. Returns whether it was.
pub(crate) fn stop_sent(message: u32) -> bool {
    if message != WM_STOP {
        return false;
    }
    // SAFETY: no pointers; it only marks this thread's queue as quitting,
    // which needs no room in it.
    unsafe { PostQuitMessage(0) };
    true
}

/// A thread with a message queue, until stopped.
pub(crate) struct MessageThread {
    id: u32,
    /// The thread's window, if it has one, as an address: another way to
    /// reach it when its queue is full.
    window: Option<usize>,
    /// Set when the thread is to stop, which its loop checks after every
    /// message.
    stopping: Arc<AtomicBool>,
    thread: Option<Joinable>,
}

impl MessageThread {
    /// Starts a thread called `name` that makes its message queue, runs
    /// `start` for its state and window (if any), and then serves its
    /// thread messages with `serve` and dispatches its window messages,
    /// until stopped; then it runs `finish`. Returns once `start` has run,
    /// with its error if it failed.
    pub(crate) fn spawn<S: 'static>(
        name: &str,
        start: impl FnOnce() -> Result<(S, Option<HWND>), String> + Send + 'static,
        mut serve: impl FnMut(&mut S, &MSG) + Send + 'static,
        finish: impl FnOnce(S) + Send + 'static,
    ) -> Result<MessageThread, String> {
        let stopping = Arc::new(AtomicBool::new(false));
        let seen = stopping.clone();
        let (started, answer) = mpsc::channel();
        let thread = Joinable::spawn(name, move || {
            let mut message = MSG::default();
            // Makes the thread's message queue, so posts to it are not lost.
            // SAFETY: `message` is a valid, writable MSG for the call.
            let _ = unsafe { PeekMessageW(&mut message, None, WM_USER, WM_USER, PM_NOREMOVE) };
            let (mut state, window) = match start() {
                Ok(started) => started,
                Err(problem) => {
                    let _ = started.send(Err(problem));
                    return;
                }
            };
            // SAFETY: no arguments; it only reads the calling thread's id.
            let id = unsafe { GetCurrentThreadId() };
            let window = window.map(|window| window.0 as usize);
            if started.send(Ok((id, window))).is_ok() {
                loop {
                    // SAFETY: `message` is a valid, writable MSG for the
                    // call; with no window it receives all of this thread's
                    // messages.
                    if unsafe { GetMessageW(&mut message, None, 0, 0) }.0 <= 0 {
                        break;
                    }
                    if seen.load(Ordering::SeqCst) {
                        break;
                    }
                    if message.hwnd.is_invalid() {
                        if message.message == WM_STOP {
                            break;
                        }
                        serve(&mut state, &message);
                    } else {
                        // SAFETY: a message this thread's queue returned.
                        unsafe { DispatchMessageW(&message) };
                    }
                }
            }
            finish(state);
        })
        .map_err(|error| error.to_string())?;
        match answer.recv() {
            Ok(Ok((id, window))) => Ok(MessageThread {
                id,
                window,
                stopping,
                thread: Some(thread),
            }),
            Ok(Err(problem)) => {
                thread.join();
                Err(problem)
            }
            Err(_) => {
                thread.join();
                Err(format!("the {name} thread did not start"))
            }
        }
    }

    /// Posts `message` to the thread, trying again for a moment while its
    /// queue is full; false if it could not.
    pub(crate) fn post(&self, message: u32) -> bool {
        for attempt in 0..POST_TRIES {
            if attempt > 0 {
                std::thread::sleep(POST_WAIT);
            }
            // SAFETY: posting a message with no pointers to a thread id.
            if unsafe { PostThreadMessageW(self.id, message, WPARAM(0), LPARAM(0)) }.is_ok() {
                return true;
            }
        }
        false
    }

    /// Stops the thread and waits for it to end, at most `limit` if given:
    /// a thread stuck longer (in a call into another program) is left to end
    /// on its own, which it does once that call returns. Returns whether it
    /// ended in time.
    pub(crate) fn stop(&mut self, limit: Option<Duration>) -> bool {
        let Some(thread) = self.thread.take() else {
            return true;
        };
        self.stopping.store(true, Ordering::SeqCst);
        if !self.post(WM_STOP)
            && let Some(window) = self.window
        {
            // A sent message needs no room in the queue; the window's
            // procedure ends the loop (`stop_sent`). It returns at once.
            // SAFETY: plain values; a window that is gone fails the call.
            let _ = unsafe {
                SendNotifyMessageW(HWND(window as *mut _), WM_STOP, WPARAM(0), LPARAM(0))
            };
        }
        match limit {
            Some(limit) => thread.join_within(limit),
            None => {
                thread.join();
                true
            }
        }
    }
}

impl Drop for MessageThread {
    fn drop(&mut self) {
        self.stop(Some(Duration::from_secs(1)));
    }
}
