//! The clipboard on Windows: a clipboard format listener
//! (`AddClipboardFormatListener`) on a message-only window of a thread of
//! Pane's own (`threads::windows::MessageThread`), which receives
//! `WM_CLIPBOARDUPDATE` after every change of the clipboard, whichever
//! application made it. No permission is needed.
//!
//! On each change the thread, and only it, opens the clipboard and reads, in
//! this order: the formats applications use to say that a clipboard monitor
//! or clipboard history must not keep what they copied
//! (`ExcludeClipboardContentFromMonitorProcessing`, the older `Clipboard
//! Viewer Ignore`, `CanIncludeInClipboardHistory` and
//! `CanUploadToCloudClipboard` as a DWORD of 0); then, only if none of them
//! forbids it, the text (`CF_UNICODETEXT`); and the file name of the
//! process whose window owns the clipboard. What is kept is decided by
//! `clipboard::accept`, the same on every system. A change is read once: if
//! reading it fails (another program holds the clipboard open), the thread
//! tries again shortly, a few times, before giving up on that change.
//!
//! Reading can wait on the program that copied (a program that renders its
//! data only when asked), however long it takes. Dropping the watch never
//! waits on it for more than a moment: it tells the thread to stop and
//! leaves it to end on its own after [`STOP_WAIT`]; Pane closed the sink's
//! fence before, so what such a late read returns is dropped. A panic while
//! handling a change is caught in the window procedure and logged, and the
//! listener goes on listening.

use std::cell::RefCell;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use ::windows::Win32::Foundation::{
    CloseHandle, GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM,
};
use ::windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
    GetClipboardOwner, GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, RemoveClipboardFormatListener, SetClipboardData,
};
use ::windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use ::windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetWindowThreadProcessId, KillTimer, SetTimer, WM_CLIPBOARDUPDATE, WM_TIMER,
};
use ::windows::core::{PCWSTR, PWSTR, w};

use super::{ClipboardSystem, Content, MAX_TEXT_BYTES, Markers, Observation, Sink, Watch};
use crate::threads::windows::{MessageThread, Window, WindowClass, stop_sent};

/// `CF_UNICODETEXT`: text as UTF-16, ending with a NUL.
const CF_UNICODETEXT: u32 = 13;

/// The listener's window, which receives the clipboard's changes.
static LISTENER_CLASS: WindowClass = WindowClass::new("PaneClipboardListener", listener_procedure);
/// The window that owns what Pane puts on the clipboard.
static WRITER_CLASS: WindowClass = WindowClass::new("PaneClipboardWriter", plain_procedure);

/// How often, and how long apart, Pane tries to open the clipboard while
/// another application holds it open.
const OPEN_TRIES: u32 = 10;
const OPEN_WAIT: Duration = Duration::from_millis(20);

/// The timer that has the listener read a change again after reading it
/// failed; how long after, and how often at most.
const RETRY_TIMER: usize = 1;
const RETRY_WAIT_MS: u32 = 250;
const READ_TRIES: u32 = 5;

/// How long dropping the watch waits for the listener to end.
pub(crate) const STOP_WAIT: Duration = Duration::from_secs(1);

/// The system's clipboard on Windows.
pub struct WindowsClipboard;

impl ClipboardSystem for WindowsClipboard {
    fn unavailable(&self) -> Option<String> {
        None
    }

    fn watch(&self, sink: Arc<dyn Sink>) -> Result<Watch, String> {
        let thread = MessageThread::spawn(
            "pane-clipboard",
            move || start_listening(sink),
            // The listener gets no thread messages but its stop.
            |_, _| {},
            stop_listening,
        )
        .map_err(|problem| format!("Pane could not watch the clipboard: {problem}"))?;
        Ok(Watch::new(Listening(thread)))
    }

    fn write_text(&self, text: &str) -> Result<(), String> {
        let owner = WRITER_CLASS.message_window()?;
        write(&owner, text, &[])
        // The clipboard is closed, then the window destroyed: what was put
        // on the clipboard stays there.
    }
}

/// The listener thread, until dropped.
struct Listening(MessageThread);

impl Drop for Listening {
    fn drop(&mut self) {
        if !self.0.stop(Some(STOP_WAIT)) {
            log(
                "Pane stopped watching the clipboard while a read was still waiting on the program that copied; it ends on its own",
            );
        }
    }
}

/// What the listener's window procedure uses, on the listener thread.
struct Listener {
    sink: Arc<dyn Sink>,
    formats: Formats,
    /// The clipboard's sequence number when it was last read in full (or
    /// given up on): a change reported twice is read once.
    read_through: u32,
    /// How many times reading the latest change failed.
    failures: u32,
}

thread_local! {
    static LISTENER: RefCell<Option<Listener>> = const { RefCell::new(None) };
}

/// Writes `message` to standard error, if there is one; never what was
/// copied. Unlike `eprintln!`, it cannot panic.
fn log(message: &str) {
    let _ = writeln!(std::io::stderr(), "{message}");
}

/// The registered formats that carry an application's markers.
#[derive(Clone, Copy)]
struct Formats {
    exclude: u32,
    viewer_ignore: u32,
    history: u32,
    cloud: u32,
}

impl Formats {
    fn register() -> Formats {
        // SAFETY: each name is a valid NUL-terminated wide string.
        unsafe {
            Formats {
                exclude: RegisterClipboardFormatW(w!(
                    "ExcludeClipboardContentFromMonitorProcessing"
                )),
                viewer_ignore: RegisterClipboardFormatW(w!("Clipboard Viewer Ignore")),
                history: RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")),
                cloud: RegisterClipboardFormatW(w!("CanUploadToCloudClipboard")),
            }
        }
    }
}

/// On the listener thread: makes its window and starts listening, reporting
/// to `sink`.
fn start_listening(sink: Arc<dyn Sink>) -> Result<(Window, Option<HWND>), String> {
    let window = LISTENER_CLASS.message_window()?;
    // SAFETY: `window` is this thread's own window.
    unsafe { AddClipboardFormatListener(window.handle()) }.map_err(|error| error.message())?;
    LISTENER.with(|listener| {
        *listener.borrow_mut() = Some(Listener {
            sink,
            formats: Formats::register(),
            // SAFETY: no arguments.
            read_through: unsafe { GetClipboardSequenceNumber() },
            failures: 0,
        });
    });
    let handle = window.handle();
    Ok((window, Some(handle)))
}

/// On the listener thread, as it ends: stops listening and lets go of the
/// sink; the window is destroyed as it is dropped.
fn stop_listening(window: Window) {
    // SAFETY: `window` is this thread's own window.
    let _ = unsafe { RemoveClipboardFormatListener(window.handle()) };
    LISTENER.with(|listener| listener.borrow_mut().take());
}

/// Reads the clipboard's latest change, unless it was read already, and
/// reports it; if reading fails, tries again later.
fn observe(window: HWND, listener: &mut Listener) {
    // SAFETY: no arguments.
    if unsafe { GetClipboardSequenceNumber() } == listener.read_through {
        return;
    }
    let ticket = listener.sink.reading();
    match read(window, listener.formats) {
        Ok((sequence, observation)) => {
            listener.read_through = sequence;
            listener.failures = 0;
            listener.sink.observed(ticket, observation);
        }
        Err(problem) => {
            listener.failures += 1;
            if listener.failures < READ_TRIES {
                // SAFETY: this thread's own window; the timer is killed
                // when it fires.
                unsafe { SetTimer(Some(window), RETRY_TIMER, RETRY_WAIT_MS, None) };
                log(&format!(
                    "Pane could not read the clipboard, and tries again: {problem}"
                ));
            } else {
                // SAFETY: no arguments.
                listener.read_through = unsafe { GetClipboardSequenceNumber() };
                listener.failures = 0;
                log(&format!(
                    "Pane could not read the clipboard, and skips this change: {problem}"
                ));
            }
        }
    }
}

extern "system" fn listener_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // A panic must not unwind into Windows, which would end Pane.
    std::panic::catch_unwind(|| handle(window, message, wparam, lparam)).unwrap_or_else(|_| {
        log("Pane's clipboard listener failed on a change; it goes on listening");
        LRESULT(0)
    })
}

fn handle(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if stop_sent(message) {
        return LRESULT(0);
    }
    let changed = match message {
        WM_CLIPBOARDUPDATE => true,
        WM_TIMER if wparam.0 == RETRY_TIMER => {
            // SAFETY: this thread's own window and timer.
            let _ = unsafe { KillTimer(Some(window), RETRY_TIMER) };
            true
        }
        _ => false,
    };
    if !changed {
        // SAFETY: the arguments are those this procedure was called with.
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }
    LISTENER.with(|listener| {
        // Not borrowed already: nothing a read waits on calls back into it.
        if let Ok(mut listener) = listener.try_borrow_mut()
            && let Some(listener) = listener.as_mut()
        {
            observe(window, listener);
        }
    });
    LRESULT(0)
}

extern "system" fn plain_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: the arguments are those this procedure was called with.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// The clipboard, opened by a window of this thread until dropped.
struct OpenedClipboard;

impl OpenedClipboard {
    /// Opens the clipboard for `owner`, a window of this thread, trying
    /// again for a moment while another application holds it open.
    fn by(owner: HWND) -> Result<OpenedClipboard, String> {
        let mut last = String::new();
        for attempt in 0..OPEN_TRIES {
            if attempt > 0 {
                std::thread::sleep(OPEN_WAIT);
            }
            // SAFETY: `owner` is a window of this thread.
            match unsafe { OpenClipboard(Some(owner)) } {
                Ok(()) => return Ok(OpenedClipboard),
                Err(error) => last = error.message(),
            }
        }
        Err(format!(
            "another application keeps the clipboard open ({last})"
        ))
    }
}

impl Drop for OpenedClipboard {
    fn drop(&mut self) {
        // SAFETY: the clipboard is open by this thread.
        let _ = unsafe { CloseClipboard() };
    }
}

/// Whether `format` is on the clipboard.
fn available(format: u32) -> bool {
    // SAFETY: a plain value.
    format != 0 && unsafe { IsClipboardFormatAvailable(format) }.is_ok()
}

/// The bytes of `format`'s global memory on the open clipboard, at most
/// `limit` of them, if it has any.
fn bytes(format: u32, limit: usize) -> Option<Vec<u8>> {
    if !available(format) {
        return None;
    }
    // SAFETY: the clipboard is open; the handle stays the clipboard's.
    let handle = unsafe { GetClipboardData(format) }.ok()?;
    let memory = HGLOBAL(handle.0);
    // SAFETY: a clipboard format with global memory, locked while read and
    // read within its size.
    unsafe {
        let size = GlobalSize(memory).min(limit);
        let data = GlobalLock(memory);
        if data.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(data.cast::<u8>(), size).to_vec();
        let _ = GlobalUnlock(memory);
        Some(bytes)
    }
}

/// A DWORD format as a yes (not 0) or no (0), if it is on the clipboard.
fn flag(format: u32) -> Option<bool> {
    let bytes = bytes(format, 4)?;
    let value: [u8; 4] = bytes.try_into().ok()?;
    Some(u32::from_le_bytes(value) != 0)
}

/// Opens the clipboard and reads its markers, then its text only if they
/// allow it, and its owner; with the sequence number of what it read.
fn read(window: HWND, formats: Formats) -> Result<(u32, Observation), String> {
    let _open = OpenedClipboard::by(window)?;
    // SAFETY: no arguments. While the clipboard is open, nothing changes it.
    let sequence = unsafe { GetClipboardSequenceNumber() };
    let markers = Markers {
        exclude_from_monitoring: available(formats.exclude) || available(formats.viewer_ignore),
        include_in_history: flag(formats.history),
        upload_to_cloud: flag(formats.cloud),
    };
    let content = if !markers.allow() {
        Content::Withheld
    } else {
        // One unit more than Pane keeps is enough to know it is too long:
        // UTF-8 never has fewer bytes than UTF-16 has units.
        match bytes(CF_UNICODETEXT, (MAX_TEXT_BYTES + 1) * 2) {
            Some(bytes) => {
                let units: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_le_bytes(*pair))
                    .take_while(|unit| *unit != 0)
                    .collect();
                Content::Text(String::from_utf16_lossy(&units))
            }
            None => Content::Other,
        }
    };
    let observation = Observation {
        content,
        markers,
        source: owner_program(),
    };
    Ok((sequence, observation))
}

/// The file name of the process whose window owns the clipboard, if the
/// system says.
fn owner_program() -> Option<String> {
    // SAFETY: no arguments.
    let owner = unsafe { GetClipboardOwner() }.ok()?;
    if owner.is_invalid() {
        return None;
    }
    let mut process = 0u32;
    // SAFETY: `owner` is a window handle; `process` is writable.
    unsafe { GetWindowThreadProcessId(owner, Some(&raw mut process)) };
    if process == 0 {
        return None;
    }
    // SAFETY: plain values; the handle is closed below.
    let handle: HANDLE =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process) }.ok()?;
    let mut name = [0u16; 1024];
    let mut length = name.len() as u32;
    // SAFETY: `name` is writable for `length` units.
    let queried = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(name.as_mut_ptr()),
            &mut length,
        )
    };
    // SAFETY: opened above.
    let _ = unsafe { CloseHandle(handle) };
    queried.ok()?;
    let path = String::from_utf16_lossy(&name[..length as usize]);
    path.rsplit(['\\', '/']).next().map(str::to_owned)
}

/// Puts `text` on the clipboard, replacing what was there, with each of
/// `markers` (a registered format's name and its DWORD value), owned by
/// `owner`, a window of this thread.
fn write(owner: &Window, text: &str, markers: &[(&str, u32)]) -> Result<(), String> {
    let _open = OpenedClipboard::by(owner.handle())?;
    // SAFETY: the clipboard is open by this thread; emptying it makes
    // `owner` its owner.
    unsafe { EmptyClipboard() }.map_err(|error| error.message())?;
    let mut units: Vec<u16> = text.encode_utf16().collect();
    units.push(0);
    put(
        CF_UNICODETEXT,
        &units
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect::<Vec<u8>>(),
    )?;
    for (name, value) in markers {
        let mut wide: Vec<u16> = name.encode_utf16().collect();
        wide.push(0);
        // SAFETY: `wide` is NUL-terminated and outlives the call.
        let format = unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) };
        put(format, &value.to_le_bytes())?;
    }
    Ok(())
}

/// Puts `bytes` on the open, emptied clipboard as `format`.
fn put(format: u32, bytes: &[u8]) -> Result<(), String> {
    // SAFETY: a new moveable block of at least one byte, written within its
    // size while locked; the clipboard owns it once set, and it is freed
    // here otherwise.
    unsafe {
        let memory =
            GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).map_err(|error| error.message())?;
        let data = GlobalLock(memory);
        if data.is_null() {
            let _ = GlobalFree(Some(memory));
            return Err("could not lock the clipboard's memory".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(memory);
        if let Err(error) = SetClipboardData(format, Some(HANDLE(memory.0))) {
            let _ = GlobalFree(Some(memory));
            return Err(error.message());
        }
    }
    Ok(())
}

/// For the Windows adapter's test: putting text with markers on the
/// clipboard, as a password manager does, owned by a window of this
/// process. It replaces what was on the clipboard, which is not saved.
#[doc(hidden)]
pub mod testing {
    use super::{WRITER_CLASS, Window, write};

    /// The window of this process that owns what the test put on the
    /// clipboard, until dropped (by the thread that put it).
    pub struct Owner(#[allow(dead_code)] Window);

    /// Puts `text` on the clipboard with each of `markers`, a registered
    /// format's name and its DWORD value, owned by a window of this process
    /// while the returned owner is kept.
    pub fn set_text(text: &str, markers: &[(&str, u32)]) -> Result<Owner, String> {
        let owner = WRITER_CLASS.message_window()?;
        write(&owner, text, markers)?;
        Ok(Owner(owner))
    }
}
