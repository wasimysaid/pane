//! Clipboard history: text the user copies, which Pane keeps for a package
//! that asked for it through `pane:extension/clipboard-history`, only after
//! the user turned it on, and never while it is paused or the package does
//! not run (disabled, paused after a failure, uninstalled).
//!
//! What is kept, and what is not, is decided here, the same on every system
//! ([`accept`]): only plain text ([`Content::Text`]) of at most
//! [`MAX_TEXT_BYTES`], not blank, not marked by the application that copied
//! it as something a clipboard monitor or clipboard history must not keep
//! ([`Markers`]), and not copied from a program the user excluded
//! ([`ProgramName`]). Each package's history is typed and kept in its own
//! file ([`history`]), newest first, one item per text and at most
//! [`MAX_ITEMS`] of them; it is the package's extension data (the kind
//! `extension_data::DataKind::ClipboardHistory`, whose storage hooks
//! dispatch here).
//!
//! The system is reached through one small trait, [`ClipboardSystem`], with
//! one adapter per system, chosen by [`native`]:
//!
//! - Windows: a clipboard format listener (`AddClipboardFormatListener`,
//!   `WM_CLIPBOARDUPDATE`) on a thread of Pane's own ([`windows`]);
//! - macOS and Linux: none yet (#37, #38), so clipboard history is
//!   unavailable there and says why.
//!
//! An adapter watches the clipboard only while Pane holds the [`Watch`] it
//! returned, which Pane does exactly while some package keeps clipboard
//! history ([`Capture`]). Pane fences what it reports: once Pane stops
//! watching, or items are deleted, a change the adapter was still reading
//! is dropped, so a slow or stuck read never delays stopping and never
//! brings back what was deleted.

use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::extension_data::{ExtensionData, PackageData};

pub(crate) mod history;
#[cfg(target_os = "windows")]
mod windows;

pub use history::Item;
#[cfg(target_os = "windows")]
pub use windows::WindowsClipboard;
#[cfg(target_os = "windows")]
#[doc(hidden)]
pub use windows::testing;

/// The most items Pane keeps for a package: copying more drops the oldest.
pub const MAX_ITEMS: usize = 100;

/// The longest text Pane keeps, in bytes of UTF-8; longer text is not kept
/// at all (not cut short).
pub const MAX_TEXT_BYTES: usize = 32 * 1024;

/// The most programs a package can exclude.
pub const MAX_EXCLUDED: usize = 64;

/// The longest program name Pane accepts to exclude.
const MAX_PROGRAM_NAME: usize = 260;

/// What the application that copied something said about keeping it, as
/// the system's clipboard formats carry it. On Windows these are the
/// formats `ExcludeClipboardContentFromMonitorProcessing` (and the older
/// `Clipboard Viewer Ignore`), `CanIncludeInClipboardHistory` and
/// `CanUploadToCloudClipboard`, which password managers set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Markers {
    /// A clipboard monitor must not look at it.
    pub exclude_from_monitoring: bool,
    /// Whether it may be kept in clipboard history, if the application said.
    pub include_in_history: Option<bool>,
    /// Whether it may be synced to other devices, if the application said.
    /// Pane never syncs anything, but an application saying no is treated as
    /// saying the text is sensitive, so it is not kept either.
    pub upload_to_cloud: Option<bool>,
}

impl Markers {
    /// Whether these markers let Pane keep what was copied.
    pub fn allow(&self) -> bool {
        !self.exclude_from_monitoring
            && self.include_in_history != Some(false)
            && self.upload_to_cloud != Some(false)
    }
}

/// What was on the clipboard, as far as Pane read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    /// Plain text.
    Text(String),
    /// Nothing Pane keeps: no text (an image or files alone), or empty.
    Other,
    /// Not read, because its markers forbid keeping it.
    Withheld,
}

/// One change of the clipboard, as an adapter reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub content: Content,
    pub markers: Markers,
    /// The file name of the program that owns the clipboard, such as
    /// `notepad.exe`, if the system says which it is.
    pub source: Option<String>,
}

/// Why an observation is not kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Skip {
    /// The application marked it as not to be kept.
    Marked,
    /// It is not text.
    NotText,
    /// It is empty or only white space.
    Blank,
    /// It is longer than [`MAX_TEXT_BYTES`].
    TooLong,
    /// It was copied from this excluded program.
    Excluded(ProgramName),
}

/// The text of `observation` if Pane keeps it for a package that excluded
/// the programs `excluded`, or why not.
pub fn accept<'a>(observation: &'a Observation, excluded: &[ProgramName]) -> Result<&'a str, Skip> {
    if !observation.markers.allow() {
        return Err(Skip::Marked);
    }
    let text = match &observation.content {
        Content::Text(text) => text,
        Content::Withheld => return Err(Skip::Marked),
        Content::Other => return Err(Skip::NotText),
    };
    if let Some(source) = &observation.source
        && let Some(program) = excluded.iter().find(|program| program.names(source))
    {
        return Err(Skip::Excluded(program.clone()));
    }
    if text.trim().is_empty() {
        return Err(Skip::Blank);
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err(Skip::TooLong);
    }
    Ok(text)
}

/// A program the user excluded, by its file name, trimmed and lowercased,
/// such as `keepass.exe`. A name read from the history file is lowercased
/// too, whatever case it was written in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub struct ProgramName(String);

impl ProgramName {
    /// `name` as an excluded program, or why it is not a program's file
    /// name.
    pub fn parse(name: &str) -> Result<ProgramName, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Name the program's file, such as KeePass.exe".into());
        }
        if name.contains(['/', '\\', ':']) {
            return Err(format!(
                "“{name}” is a path: name only the program's file, such as KeePass.exe"
            ));
        }
        if name.chars().count() > MAX_PROGRAM_NAME {
            return Err(format!(
                "A program's file name has at most {MAX_PROGRAM_NAME} characters"
            ));
        }
        Ok(ProgramName::from(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the program `source` (a file name) is this one: the same
    /// file name, or the same name without its extension, ignoring case,
    /// so `KeePass` excludes `KeePass.exe`.
    pub fn names(&self, source: &str) -> bool {
        let source = source.trim().to_lowercase();
        let stem = source
            .rsplit_once('.')
            .map_or(source.as_str(), |(stem, _)| stem);
        source == self.0 || stem == self.0
    }
}

impl From<String> for ProgramName {
    fn from(name: String) -> ProgramName {
        ProgramName(name.trim().to_lowercase())
    }
}

impl From<ProgramName> for String {
    fn from(name: ProgramName) -> String {
        name.0
    }
}

/// Whether Pane keeps what is copied for a package.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaptureState {
    /// Never turned on, or turned off: nothing is kept. Every package
    /// starts so.
    #[default]
    Off,
    /// Text copied is kept while the package runs.
    On,
    /// Turned on, then paused: nothing is kept until it is resumed.
    Paused,
}

impl CaptureState {
    pub fn is_off(&self) -> bool {
        *self == CaptureState::Off
    }
}

/// Taken by an adapter just before it reads a change of the clipboard, and
/// handed back with what it read ([`Sink::observed`]): if Pane stopped
/// watching or items were deleted meanwhile, what it read is dropped. The
/// default ticket is for a sink of an adapter's own tests, which fences
/// nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ticket(u64);

/// Where an adapter reports the changes of the clipboard, from its own
/// thread.
pub trait Sink: Send + Sync + 'static {
    /// Called before the adapter reads a change.
    fn reading(&self) -> Ticket;

    /// What the adapter read with `ticket`.
    fn observed(&self, ticket: Ticket, observation: Observation);
}

/// Watching the clipboard: the adapter's listener, which stops listening
/// when this is dropped. Dropping it never waits long for a listener stuck
/// in a read; Pane stops using the listener's reports before it drops this.
pub struct Watch(#[allow(dead_code)] Box<dyn Send>);

impl Watch {
    /// A watch that stops when `listener` is dropped.
    pub fn new(listener: impl Send + 'static) -> Watch {
        Watch(Box::new(listener))
    }
}

/// The system's clipboard, as Pane watches and writes it.
pub trait ClipboardSystem: Send + Sync + 'static {
    /// Why Pane cannot watch the clipboard on this system, if it cannot.
    fn unavailable(&self) -> Option<String>;

    /// Starts watching: `sink` is told of each later change of the
    /// clipboard (not of what is on it now) until the returned watch is
    /// dropped.
    fn watch(&self, sink: Arc<dyn Sink>) -> Result<Watch, String>;

    /// Puts `text` on the clipboard, as copying it would.
    fn write_text(&self, text: &str) -> Result<(), String>;
}

/// This system's adapter: Windows' clipboard format listener, or one that
/// explains why clipboard history is unavailable here.
pub fn native() -> Arc<dyn ClipboardSystem> {
    #[cfg(target_os = "windows")]
    {
        Arc::new(WindowsClipboard)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let here = crate::platform::Platform::current()
            .map_or_else(|| std::env::consts::OS.to_string(), |p| p.to_string());
        Arc::new(Unavailable(format!(
            "Not available on {here}: Pane watches the clipboard only on Windows so far"
        )))
    }
}

/// A clipboard Pane does not watch, for a launcher given none.
pub fn none() -> Arc<dyn ClipboardSystem> {
    Arc::new(Unavailable(
        "Not available: this Pane does not watch the clipboard".into(),
    ))
}

/// A system where clipboard history is unavailable, saying why.
struct Unavailable(String);

impl ClipboardSystem for Unavailable {
    fn unavailable(&self) -> Option<String> {
        Some(self.0.clone())
    }

    fn watch(&self, _sink: Arc<dyn Sink>) -> Result<Watch, String> {
        Err(self.0.clone())
    }

    fn write_text(&self, _text: &str) -> Result<(), String> {
        Err(self.0.clone())
    }
}

/// Milliseconds since the Unix epoch, now.
pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Watches the clipboard exactly while some package keeps clipboard
/// history: its capture is on and it runs. Every change to either calls
/// [`Capture::reconcile`] (see [`ExtensionData::set_changed`]), so turning
/// history off, pausing it or disabling, pausing or uninstalling the
/// package stops the watch at once, and turning it on or enabling the
/// package starts it again.
pub(crate) struct Capture {
    system: Arc<dyn ClipboardSystem>,
    data: ExtensionData,
    watching: Mutex<Watching>,
}

#[derive(Default)]
struct Watching {
    /// The adapter's watch, with the fence of its reports.
    current: Option<(Watch, Arc<Fence>)>,
    /// Why the adapter could not start watching, until it next can.
    problem: Option<String>,
}

/// Lets a watch's reports through until it is closed. A report is kept
/// while holding it, so once [`Fence::close`] returns, none is kept any
/// more.
struct Fence(Mutex<bool>);

impl Fence {
    fn lock(&self) -> MutexGuard<'_, bool> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn close(&self) {
        *self.lock() = false;
    }
}

/// Where a watch reports: it keeps what the packages capturing now accept.
struct CaptureSink {
    data: ExtensionData,
    fence: Arc<Fence>,
}

impl Sink for CaptureSink {
    fn reading(&self) -> Ticket {
        Ticket(self.data.clipboard_history().deletions())
    }

    fn observed(&self, ticket: Ticket, observation: Observation) {
        let open = self.fence.lock();
        if !*open {
            return;
        }
        let running = self.data.running_owners();
        let now = now();
        self.data.clipboard_history().capture(ticket.0, |packages| {
            let mut changed = false;
            for (owner, history) in packages.iter_mut() {
                if history.capture != CaptureState::On || !running.contains(owner) {
                    continue;
                }
                if let Ok(text) = accept(&observation, &history.excluded) {
                    history.add(text, observation.source.as_deref(), now);
                    changed = true;
                }
            }
            changed
        });
        drop(open);
    }
}

impl Capture {
    /// Keeps clipboard history for the packages of `data` through
    /// `system`, starting to watch now if one keeps it.
    pub fn start(system: Arc<dyn ClipboardSystem>, data: ExtensionData) -> Arc<Capture> {
        let capture = Arc::new(Capture {
            system,
            data: data.clone(),
            watching: Mutex::new(Watching::default()),
        });
        let weak: Weak<Capture> = Arc::downgrade(&capture);
        data.set_changed(Arc::new(move || {
            if let Some(capture) = weak.upgrade() {
                capture.reconcile();
            }
        }));
        capture.reconcile();
        capture
    }

    /// The system's clipboard.
    pub fn system(&self) -> &Arc<dyn ClipboardSystem> {
        &self.system
    }

    /// Why Pane does not watch the clipboard now although it should, or
    /// cannot on this system.
    pub fn problem(&self) -> Option<String> {
        self.system
            .unavailable()
            .or_else(|| self.lock().problem.clone())
    }

    /// Whether some package keeps clipboard history now: its capture is on
    /// and its code may run.
    fn capturing(&self) -> bool {
        let running = self.data.running_owners();
        self.data
            .clipboard_history()
            .capturing_owners()
            .iter()
            .any(|owner| running.contains(owner))
    }

    /// Starts or stops watching, as the packages' capture states and
    /// generations now require. Stopping closes the watch's fence, then
    /// drops it with nothing locked, so it never waits for a read in
    /// progress.
    pub fn reconcile(&self) {
        let wanted = self.system.unavailable().is_none() && self.capturing();
        let stopping = {
            let mut watching = self.lock();
            if wanted {
                if watching.current.is_none() {
                    let fence = Arc::new(Fence(Mutex::new(true)));
                    let sink = Arc::new(CaptureSink {
                        data: self.data.clone(),
                        fence: fence.clone(),
                    });
                    match self.system.watch(sink) {
                        Ok(watch) => {
                            watching.current = Some((watch, fence));
                            watching.problem = None;
                        }
                        Err(problem) => watching.problem = Some(problem),
                    }
                }
                None
            } else {
                watching.problem = None;
                watching.current.take()
            }
        };
        if let Some((watch, fence)) = stopping {
            fence.close();
            drop(watch);
        }
    }

    fn lock(&self) -> MutexGuard<'_, Watching> {
        self.watching
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A package's clipboard history as its commands see it.
pub(crate) struct Status {
    pub capture: CaptureState,
    /// Why Pane does not watch the clipboard although it should, or cannot.
    pub problem: Option<String>,
    pub excluded: Vec<ProgramName>,
    pub items: usize,
}

/// What a command of the package with `data` does with its clipboard
/// history through `capture` (none: this Pane does not watch the
/// clipboard). Code whose generation ended reads and changes nothing more.
pub(crate) struct Commands<'a> {
    pub data: &'a PackageData,
    pub capture: Option<Arc<Capture>>,
}

impl Commands<'_> {
    fn unavailable() -> String {
        none().unavailable().unwrap_or_default()
    }

    fn system(&self) -> Result<Arc<dyn ClipboardSystem>, String> {
        self.capture
            .as_ref()
            .map(|capture| capture.system().clone())
            .ok_or_else(Commands::unavailable)
    }

    pub fn status(&self) -> Result<Status, String> {
        let history = self.data.clipboard_history()?.get(self.data.owner())?;
        let problem = match &self.capture {
            Some(capture) => capture.problem(),
            None => Some(Commands::unavailable()),
        };
        Ok(Status {
            capture: history.capture,
            problem,
            excluded: history.excluded,
            items: history.items.len(),
        })
    }

    pub fn set_capture(&self, state: CaptureState) -> Result<(), String> {
        let system = self.system()?;
        if state == CaptureState::On
            && let Some(reason) = system.unavailable()
        {
            return Err(reason);
        }
        self.update(|history| {
            history.capture = state;
            Ok(())
        })
    }

    pub fn set_excluded(&self, programs: &[String]) -> Result<(), String> {
        self.update(|history| history.set_excluded(programs))
    }

    /// The kept items, newest first.
    pub fn items(&self) -> Result<Vec<Item>, String> {
        Ok(self.data.clipboard_history()?.get(self.data.owner())?.items)
    }

    /// Puts the kept item `id` on the clipboard again.
    pub fn copy(&self, id: &str) -> Result<(), String> {
        let system = self.system()?;
        let item = self
            .items()?
            .into_iter()
            .find(|item| item.id.to_string() == id)
            .ok_or("that item is no longer kept")?;
        system.write_text(&item.text)
    }

    /// Deletes every kept item; returns how many there were. A change the
    /// adapter was reading meanwhile is not kept.
    pub fn clear(&self) -> Result<usize, String> {
        self.update(|history| Ok(history.clear()))
    }

    fn update<R>(
        &self,
        change: impl FnOnce(&mut history::PackageHistory) -> Result<R, String>,
    ) -> Result<R, String> {
        let (answer, capture_changed) = self
            .data
            .clipboard_history()?
            .update(self.data.owner(), change)?;
        if capture_changed {
            self.data.changed();
        }
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(text: &str) -> Observation {
        Observation {
            content: Content::Text(text.into()),
            markers: Markers::default(),
            source: None,
        }
    }

    fn from(text: &str, source: &str) -> Observation {
        Observation {
            source: Some(source.into()),
            ..self::text(text)
        }
    }

    #[test]
    fn plain_text_is_kept() {
        assert_eq!(accept(&text("hello"), &[]), Ok("hello"));
        assert_eq!(accept(&text("  two words \n"), &[]), Ok("  two words \n"));
    }

    #[test]
    fn marked_text_is_not_kept() {
        let marked = |markers: Markers| Observation {
            markers,
            ..text("hunter2")
        };
        let excluded = Markers {
            exclude_from_monitoring: true,
            ..Markers::default()
        };
        let no_history = Markers {
            include_in_history: Some(false),
            ..Markers::default()
        };
        let no_cloud = Markers {
            upload_to_cloud: Some(false),
            ..Markers::default()
        };
        for markers in [excluded, no_history, no_cloud] {
            assert_eq!(accept(&marked(markers), &[]), Err(Skip::Marked));
        }
        // Saying yes changes nothing.
        let allowed = Markers {
            include_in_history: Some(true),
            upload_to_cloud: Some(true),
            ..Markers::default()
        };
        assert_eq!(accept(&marked(allowed), &[]), Ok("hunter2"));
        let withheld = Observation {
            content: Content::Withheld,
            ..text("")
        };
        assert_eq!(accept(&withheld, &[]), Err(Skip::Marked));
    }

    #[test]
    fn other_blank_and_long_content_is_not_kept() {
        let other = Observation {
            content: Content::Other,
            ..text("")
        };
        assert_eq!(accept(&other, &[]), Err(Skip::NotText));
        assert_eq!(accept(&text(""), &[]), Err(Skip::Blank));
        assert_eq!(accept(&text(" \t\r\n"), &[]), Err(Skip::Blank));
        let longest = "a".repeat(MAX_TEXT_BYTES);
        assert_eq!(accept(&text(&longest), &[]), Ok(longest.as_str()));
        let longer = format!("{longest}é");
        assert_eq!(accept(&text(&longer), &[]), Err(Skip::TooLong));
    }

    #[test]
    fn text_from_an_excluded_program_is_not_kept() {
        let keepass = ProgramName::parse(" KeePass.exe ").unwrap();
        let excluded = vec![keepass.clone(), ProgramName::from("1Password".to_string())];
        assert_eq!(
            accept(&from("secret", "KEEPASS.EXE"), &excluded),
            Err(Skip::Excluded(keepass))
        );
        assert_eq!(
            accept(&from("secret", "1Password.exe"), &excluded),
            Err(Skip::Excluded(ProgramName::from("1password".to_string())))
        );
        assert_eq!(accept(&from("note", "notepad.exe"), &excluded), Ok("note"));
        // A program the system does not name is not excluded.
        assert_eq!(accept(&text("note"), &excluded), Ok("note"));
        // Only the whole name counts.
        assert_eq!(
            accept(&from("note", "keepassxc.exe"), &excluded),
            Ok("note")
        );
    }

    #[test]
    fn a_program_is_named_by_its_file_in_lower_case() {
        assert_eq!(
            ProgramName::parse("KeePass.exe").unwrap().as_str(),
            "keepass.exe"
        );
        assert!(ProgramName::parse("  ").is_err());
        assert!(ProgramName::parse(r"C:\Program Files\KeePass.exe").is_err());
        assert!(ProgramName::parse(&"a".repeat(261)).is_err());
        // However it was written in the file.
        let read: Vec<ProgramName> =
            serde_json::from_str(r#"["KeePass.EXE", " Bitwarden "]"#).unwrap();
        assert_eq!(
            read,
            [
                ProgramName::from("keepass.exe".to_string()),
                ProgramName::from("bitwarden".to_string())
            ]
        );
        assert!(read[0].names("KEEPASS.exe") && read[1].names("Bitwarden.exe"));
    }
}
