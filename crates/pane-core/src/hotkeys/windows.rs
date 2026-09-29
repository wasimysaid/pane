//! Global hotkeys on Windows: `RegisterHotKey` on a thread of Pane's own,
//! whose message loop receives `WM_HOTKEY` whichever window has focus. A
//! shortcut another application registered already is refused with
//! `ERROR_HOTKEY_ALREADY_REGISTERED`, which is how a conflict shows; Windows
//! also refuses some of its own shortcuts that way. No permission is needed.
//!
//! A hotkey belongs to the thread that registered it, so registering and
//! releasing are done by that thread: the caller queues the request, wakes
//! the thread with a thread message and waits for its answer. Dropping the
//! adapter ends the thread, which releases its hotkeys as it ends. The
//! thread is a `threads::windows::MessageThread`, like the clipboard
//! listener's.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use ::windows::Win32::Foundation::ERROR_HOTKEY_ALREADY_REGISTERED;
use ::windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};
use ::windows::Win32::UI::WindowsAndMessaging::{MSG, WM_HOTKEY};
use ::windows::core::HRESULT;

use super::{HotkeyError, Hotkeys, PressSender, Shortcut};
use crate::threads::windows::{MessageThread, WM_WAKE};

enum Request {
    Register(Shortcut, mpsc::Sender<Result<(), HotkeyError>>),
    Unregister(Shortcut, mpsc::Sender<()>),
}

type Requests = Arc<Mutex<VecDeque<Request>>>;

/// The adapter: a thread that registers the hotkeys and receives their
/// presses.
pub struct WindowsHotkeys {
    thread: MessageThread,
    requests: Requests,
}

/// The virtual-key code of `key` (see `Shortcut::key`).
fn virtual_key(key: &str) -> Option<u32> {
    match key.as_bytes() {
        [c @ b'a'..=b'z'] => Some(u32::from(c.to_ascii_uppercase())),
        [c @ b'0'..=b'9'] => Some(u32::from(*c)),
        _ if key == "space" => Some(0x20),
        _ => {
            let number: u32 = key.strip_prefix('f')?.parse().ok()?;
            // VK_F1 is 0x70.
            (1..=12).contains(&number).then(|| 0x70 + number - 1)
        }
    }
}

fn modifiers(shortcut: &Shortcut) -> HOT_KEY_MODIFIERS {
    // Holding the keys reports one press, not one per repeat.
    let mut modifiers = MOD_NOREPEAT;
    if shortcut.control() {
        modifiers |= MOD_CONTROL;
    }
    if shortcut.alt() {
        modifiers |= MOD_ALT;
    }
    if shortcut.shift() {
        modifiers |= MOD_SHIFT;
    }
    if shortcut.super_key() {
        modifiers |= MOD_WIN;
    }
    modifiers
}

impl WindowsHotkeys {
    /// Starts the hotkey thread.
    pub fn start(presses: PressSender) -> Result<WindowsHotkeys, String> {
        let requests: Requests = Arc::default();
        let served = requests.clone();
        let thread = MessageThread::spawn(
            "pane-hotkeys",
            || Ok((Registered::default(), None)),
            move |registered, message| registered.serve(message, &served, &presses),
            Registered::release_all,
        )?;
        Ok(WindowsHotkeys { thread, requests })
    }

    /// Queues `request` and wakes the thread; false if it is gone.
    fn send(&self, request: Request) -> bool {
        self.requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push_back(request);
        self.thread.post(WM_WAKE)
    }
}

impl Hotkeys for WindowsHotkeys {
    fn unavailable(&self) -> Option<String> {
        None
    }

    fn register(&self, shortcut: &Shortcut) -> Result<(), HotkeyError> {
        let (answer, answered) = mpsc::channel();
        if !self.send(Request::Register(shortcut.clone(), answer)) {
            return Err(HotkeyError::Refused("the hotkey thread stopped".into()));
        }
        answered
            .recv()
            .unwrap_or_else(|_| Err(HotkeyError::Refused("the hotkey thread stopped".into())))
    }

    fn unregister(&self, shortcut: &Shortcut) {
        let (answer, answered) = mpsc::channel();
        if self.send(Request::Unregister(shortcut.clone(), answer)) {
            let _ = answered.recv();
        }
    }
}

impl Drop for WindowsHotkeys {
    /// Ends the thread, which releases every hotkey as it ends. It never
    /// waits on another program, so this waits until it has.
    fn drop(&mut self) {
        self.thread.stop(None);
    }
}

/// The hotkeys the thread registered, by their id.
#[derive(Default)]
struct Registered {
    shortcuts: HashMap<i32, Shortcut>,
    last_id: i32,
}

impl Registered {
    fn release(&mut self, id: i32) {
        // SAFETY: `id` was registered by this thread with no window.
        let _ = unsafe { UnregisterHotKey(None, id) };
        self.shortcuts.remove(&id);
    }

    fn release_all(mut self) {
        let ids: Vec<i32> = self.shortcuts.keys().copied().collect();
        for id in ids {
            self.release(id);
        }
    }

    /// Reports a hotkey's press, or serves the queued requests.
    fn serve(&mut self, message: &MSG, requests: &Mutex<VecDeque<Request>>, presses: &PressSender) {
        match message.message {
            WM_HOTKEY => {
                if let Some(shortcut) = self.shortcuts.get(&(message.wParam.0 as i32)) {
                    presses.send(shortcut.clone());
                }
            }
            WM_WAKE => loop {
                let request = requests
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .pop_front();
                match request {
                    None => break,
                    Some(Request::Register(shortcut, answer)) => {
                        let _ = answer.send(self.register(shortcut));
                    }
                    Some(Request::Unregister(shortcut, answer)) => {
                        let ids: Vec<i32> = self
                            .shortcuts
                            .iter()
                            .filter(|(_, done)| **done == shortcut)
                            .map(|(id, _)| *id)
                            .collect();
                        for id in ids {
                            self.release(id);
                        }
                        let _ = answer.send(());
                    }
                }
            },
            _ => {}
        }
    }

    fn register(&mut self, shortcut: Shortcut) -> Result<(), HotkeyError> {
        if self.shortcuts.values().any(|done| *done == shortcut) {
            return Ok(());
        }
        let Some(key) = virtual_key(shortcut.key()) else {
            return Err(HotkeyError::Refused(format!(
                "{shortcut} has no Windows key"
            )));
        };
        let id = self.last_id + 1;
        let taken = HRESULT::from_win32(ERROR_HOTKEY_ALREADY_REGISTERED.0);
        // SAFETY: plain values; with no window the hotkey belongs to this
        // thread, whose loop receives it.
        match unsafe { RegisterHotKey(None, id, modifiers(&shortcut), key) } {
            Ok(()) => {
                self.last_id = id;
                self.shortcuts.insert(id, shortcut);
                Ok(())
            }
            Err(error) if error.code() == taken => Err(HotkeyError::Taken),
            Err(error) => Err(HotkeyError::Refused(error.message())),
        }
    }
}
