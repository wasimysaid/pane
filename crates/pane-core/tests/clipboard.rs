//! Clipboard history through the launcher's public interface, with a fake
//! system clipboard, for the Clipboard History default extension (Rust) and
//! the JavaScript and TypeScript clipboard samples alike: nothing is kept
//! until the user turns it on in the command; then Pane watches the
//! clipboard and keeps plain text, except what is marked as not to be kept
//! or comes from an excluded program; pausing, turning it off, disabling
//! and uninstalling stop the watch at once, and a restart watches again only
//! where history is on and the package enabled. A change the system was
//! still reading when the watch stopped or the history was cleared is not
//! kept, and stopping never waits for it. The packages are the ones `cargo
//! xtask guests` assembles in `target/guests/packages`; since only Windows
//! has a clipboard adapter so far, the tests install a copy whose manifest
//! declares every system, so the same checks run everywhere. The system's
//! real clipboard is never used here.

#[path = "support/platforms.rs"]
mod platforms;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::executor::block_on;
use pane_core::clipboard::{
    ClipboardSystem, Content, MAX_ITEMS, MAX_TEXT_BYTES, Markers, Observation, Sink, Ticket, Watch,
};
use pane_core::{Launcher, Runtime, Screen, Status, Unavailable};
use serde_json::Value;
use tempfile::TempDir;

const MANAGE_ROW: &str = "Manage extensions…";
const TURN_ON: &str = "Turn on clipboard history";
const TURN_OFF: &str = "Turn off clipboard history";
const PAUSE: &str = "Pause clipboard history";
const RESUME: &str = "Resume clipboard history";
const CLEAR: &str = "Clear clipboard history";
const EXCLUDE: &str = "Exclude a program";

/// A package with a clipboard history command, in one language.
struct Fixture {
    /// The assembled package under `target/guests/packages`.
    package: &'static str,
    component: &'static str,
    /// Its package's title.
    title: &'static str,
    /// Its command's title.
    command: &'static str,
}

const RUST: Fixture = Fixture {
    package: "clipboard-history",
    component: "clipboard_history.wasm",
    title: "Clipboard History",
    command: "Clipboard History",
};
const JAVASCRIPT: Fixture = Fixture {
    package: "sample-clipboard-js",
    component: "sample_clipboard_js.wasm",
    title: "JavaScript clipboard sample",
    command: "Clipboard history (JavaScript)",
};
const TYPESCRIPT: Fixture = Fixture {
    package: "sample-clipboard-ts",
    component: "sample_clipboard_ts.wasm",
    title: "TypeScript clipboard sample",
    command: "Clipboard history (TypeScript)",
};

fn built(path: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/guests")
        .join(path);
    assert!(
        path.exists(),
        "{} is missing; run `cargo xtask guests`",
        path.display()
    );
    path
}

#[derive(Default)]
struct Clipboard {
    /// Where changes go while Pane watches.
    sink: Option<Arc<dyn Sink>>,
    /// How many times Pane started watching.
    started: usize,
    /// What Pane put on the clipboard.
    written: Vec<String>,
}

/// A system clipboard that records what Pane does with it, and reports
/// only the changes a test makes.
#[derive(Clone, Default)]
struct FakeClipboard {
    inner: Arc<Mutex<Clipboard>>,
    unavailable: Option<String>,
}

/// Watching the fake clipboard, until dropped.
struct FakeWatch(Arc<Mutex<Clipboard>>);

impl Drop for FakeWatch {
    fn drop(&mut self) {
        self.0.lock().unwrap().sink = None;
    }
}

/// A change the fake system has begun reading, as a slow read would be.
struct Reading {
    sink: Arc<dyn Sink>,
    ticket: Ticket,
}

impl Reading {
    /// Finishes the read with `text`, as a system would report it however
    /// late.
    fn finish(self, text: &str) {
        self.sink.observed(self.ticket, copied(text, None));
    }
}

fn copied(text: &str, source: Option<&str>) -> Observation {
    Observation {
        content: Content::Text(text.into()),
        markers: Markers::default(),
        source: source.map(str::to_owned),
    }
}

impl FakeClipboard {
    fn watching(&self) -> bool {
        self.inner.lock().unwrap().sink.is_some()
    }

    fn started(&self) -> usize {
        self.inner.lock().unwrap().started
    }

    fn written(&self) -> Vec<String> {
        self.inner.lock().unwrap().written.clone()
    }

    /// Begins reading a change, if Pane watches.
    fn begin_read(&self) -> Option<Reading> {
        let sink = self.inner.lock().unwrap().sink.clone()?;
        let ticket = sink.reading();
        Some(Reading { sink, ticket })
    }

    /// Reports `observation` as a change of the clipboard, if Pane
    /// watches; returns whether it did.
    fn change(&self, observation: Observation) -> bool {
        match self.begin_read() {
            Some(reading) => {
                reading.sink.observed(reading.ticket, observation);
                true
            }
            None => false,
        }
    }

    /// Reports `text` copied from `source`.
    fn copy(&self, text: &str, source: Option<&str>) -> bool {
        self.change(copied(text, source))
    }
}

impl ClipboardSystem for FakeClipboard {
    fn unavailable(&self) -> Option<String> {
        self.unavailable.clone()
    }

    fn watch(&self, sink: Arc<dyn Sink>) -> Result<Watch, String> {
        if let Some(reason) = &self.unavailable {
            return Err(reason.clone());
        }
        let mut inner = self.inner.lock().unwrap();
        assert!(inner.sink.is_none(), "Pane watches the clipboard once");
        inner.sink = Some(sink);
        inner.started += 1;
        Ok(Watch::new(FakeWatch(self.inner.clone())))
    }

    fn write_text(&self, text: &str) -> Result<(), String> {
        self.inner.lock().unwrap().written.push(text.into());
        // As on a real system, writing is a change Pane sees.
        self.copy(text, Some("pane.exe"));
        Ok(())
    }
}

/// Pane's data location and the package's source folder for one test,
/// which outlive restarts.
struct Pane {
    fixture: &'static Fixture,
    data: TempDir,
    source: TempDir,
    clipboard: FakeClipboard,
}

impl Pane {
    fn new(fixture: &'static Fixture) -> Pane {
        Pane::with(fixture, FakeClipboard::default())
    }

    fn with(fixture: &'static Fixture, clipboard: FakeClipboard) -> Pane {
        let source = tempfile::tempdir().unwrap();
        copy_package(fixture, source.path(), true);
        Pane {
            fixture,
            data: tempfile::tempdir().unwrap(),
            source,
            clipboard,
        }
    }

    fn folder(&self) -> &Path {
        self.source.path()
    }

    /// Starts Pane on this data location, as after a restart.
    fn start(&self) -> Launcher {
        Launcher::with_packages(
            Runtime::start(),
            vec![],
            self.data.path().join("extensions"),
        )
        .with_clipboard(Arc::new(self.clipboard.clone()))
    }

    /// Starts Pane with the package installed.
    fn installed(&self) -> Launcher {
        let launcher = self.start();
        install(&launcher, self.folder());
        launcher
    }

    fn history_path(&self) -> PathBuf {
        self.data
            .path()
            .join("extensions")
            .join("clipboard-history.json")
    }

    /// Every package's clipboard history as Pane keeps it on disk.
    fn history_file(&self) -> Option<Value> {
        let text = fs::read_to_string(self.history_path()).ok()?;
        Some(serde_json::from_str(&text).unwrap())
    }

    /// The texts kept on disk, newest first, for the only package that
    /// keeps any.
    fn kept_on_disk(&self) -> Vec<String> {
        let Some(file) = self.history_file() else {
            return Vec::new();
        };
        assert_eq!(file["version"], 1);
        let packages = file["packages"].as_object().unwrap();
        assert!(packages.len() <= 1, "{packages:?}");
        let Some(history) = packages.values().next() else {
            return Vec::new();
        };
        history["items"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| item["text"].as_str().unwrap().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Opens the command from root search, showing its view as it is now.
    fn open(&self, launcher: &Launcher) {
        launcher.back();
        launcher.back();
        block_on(launcher.set_query("clipboard"));
        select_title(launcher, self.fixture.command);
        block_on(launcher.activate_selected());
        assert_eq!(
            launcher.view().screen,
            Screen::Command,
            "{:?}",
            launcher.view().status
        );
    }

    /// The kept texts the command lists, newest first: the rows after its
    /// controls.
    fn listed(&self, launcher: &Launcher) -> Vec<String> {
        self.open(launcher);
        launcher
            .view()
            .rows
            .into_iter()
            .filter(|row| {
                row.subtitle
                    .as_deref()
                    .is_some_and(|subtitle| subtitle.ends_with("Enter copies it"))
            })
            .map(|row| row.title)
            .collect()
    }

    fn turn_on(&self, launcher: &Launcher) {
        self.open(launcher);
        assert_eq!(run(launcher, TURN_ON), result("Clipboard history is on"));
    }

    /// From root search, uninstalls the package with the confirmation row
    /// `choice`, returning the confirmation's details.
    fn uninstall(&self, launcher: &Launcher, choice: &str) -> Vec<String> {
        launcher.back();
        launcher.back();
        select_title(launcher, MANAGE_ROW);
        block_on(launcher.activate_selected());
        select_title(launcher, &format!("Uninstall {}", self.fixture.title));
        block_on(launcher.activate_selected());
        assert!(matches!(launcher.view().screen, Screen::Confirm { .. }));
        let details = launcher.view().details().to_vec();
        select_title(launcher, choice);
        block_on(launcher.activate_selected());
        assert!(
            matches!(launcher.view().status, Status::Result(_)),
            "{:?}",
            launcher.view().status
        );
        details
    }
}

fn install(launcher: &Launcher, folder: &Path) {
    block_on(launcher.install_package(folder));
    assert!(
        matches!(launcher.view().status, Status::Result(_)),
        "{:?}",
        launcher.view().status
    );
    launcher.back();
}

/// Copies the assembled package of `fixture` into `folder`; with
/// `everywhere`, its command declares every system, so it is available
/// where these tests run.
fn copy_package(fixture: &Fixture, folder: &Path, everywhere: bool) {
    let package = built(&format!("packages/{}", fixture.package));
    fs::copy(
        package.join(fixture.component),
        folder.join(fixture.component),
    )
    .unwrap();
    let manifest = fs::read_to_string(package.join("pane.json")).unwrap();
    let mut manifest: Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(
        manifest["commands"][0]["platforms"],
        serde_json::json!(["windows"])
    );
    if everywhere {
        manifest["commands"][0]["platforms"] = serde_json::json!(["windows", "macos", "linux"]);
    }
    fs::write(folder.join("pane.json"), manifest.to_string()).unwrap();
}

fn titles(launcher: &Launcher) -> Vec<String> {
    launcher
        .view()
        .rows
        .into_iter()
        .map(|row| row.title)
        .collect()
}

fn subtitle(launcher: &Launcher, title: &str) -> String {
    launcher
        .view()
        .rows
        .into_iter()
        .find(|row| row.title == title)
        .and_then(|row| row.subtitle)
        .unwrap_or_else(|| panic!("no row {title:?} in {:?}", titles(launcher)))
}

fn select_title(launcher: &Launcher, title: &str) {
    let index = titles(launcher)
        .iter()
        .position(|row| row == title)
        .unwrap_or_else(|| panic!("no row {title:?} in {:?}", titles(launcher)));
    launcher.select(index);
}

/// Runs the item titled `title` of the open command.
fn run(launcher: &Launcher, title: &str) -> Status {
    select_title(launcher, title);
    block_on(launcher.activate_selected());
    launcher.view().status
}

fn result(text: &str) -> Status {
    Status::Result(text.into())
}

fn nothing_is_watched_or_kept_until_history_is_turned_on(fixture: &'static Fixture) {
    let pane = Pane::new(fixture);
    let launcher = pane.installed();
    assert!(!pane.clipboard.watching());

    pane.open(&launcher);
    assert_eq!(titles(&launcher), [TURN_ON, EXCLUDE]);
    assert_eq!(
        subtitle(&launcher, TURN_ON),
        "Off · Pane keeps nothing you copy until you turn it on. Once on, it keeps the text you \
         copy on this computer; nothing is sent anywhere"
    );
    assert!(!pane.clipboard.copy("before", Some("notepad.exe")));
    assert_eq!(pane.clipboard.started(), 0);

    assert_eq!(run(&launcher, TURN_ON), result("Clipboard history is on"));
    assert!(pane.clipboard.watching());
    // Pressing it again before the view is shown anew changes nothing.
    assert_eq!(run(&launcher, TURN_ON), result("Clipboard history is on"));
    assert_eq!(pane.clipboard.started(), 1);
    assert!(pane.clipboard.copy("hello", Some("notepad.exe")));
    assert!(pane.clipboard.copy("second line\nand more", None));

    assert_eq!(pane.listed(&launcher), ["second line", "hello"]);
    assert_eq!(titles(&launcher)[..4], [PAUSE, TURN_OFF, EXCLUDE, CLEAR]);
    assert_eq!(
        subtitle(&launcher, "hello"),
        "just now · from notepad.exe · Enter copies it"
    );
    assert_eq!(
        subtitle(&launcher, "second line"),
        "just now · 2 lines · Enter copies it"
    );
    assert_eq!(
        subtitle(&launcher, PAUSE),
        "On · 2 items kept · Text you copy is kept on this computer"
    );
    assert_eq!(pane.kept_on_disk(), ["second line\nand more", "hello"]);
    let file = fs::read_to_string(pane.history_path()).unwrap();
    assert!(!file.contains("before"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(pane.history_path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

fn turning_history_off_stops_the_watch_and_keeps_the_items(fixture: &'static Fixture) {
    let pane = Pane::new(fixture);
    let launcher = pane.installed();
    pane.turn_on(&launcher);
    pane.clipboard.copy("kept", None);
    pane.open(&launcher);
    assert_eq!(
        subtitle(&launcher, TURN_OFF),
        "Stops keeping what you copy; the kept items stay until you clear them"
    );
    assert_eq!(run(&launcher, TURN_OFF), result("Clipboard history is off"));
    assert!(!pane.clipboard.watching());
    assert!(!pane.clipboard.copy("while off", None));
    // Off after a restart too, with its item still listed.
    drop(launcher);
    let launcher = pane.start();
    assert!(!pane.clipboard.watching());
    assert_eq!(pane.listed(&launcher), ["kept"]);
    assert_eq!(titles(&launcher)[..3], [TURN_ON, EXCLUDE, CLEAR]);
    // Paused, it can be turned off too.
    assert_eq!(run(&launcher, TURN_ON), result("Clipboard history is on"));
    pane.open(&launcher);
    assert_eq!(run(&launcher, PAUSE), result("Clipboard history is paused"));
    pane.open(&launcher);
    assert_eq!(titles(&launcher)[..2], [RESUME, TURN_OFF]);
    assert_eq!(run(&launcher, TURN_OFF), result("Clipboard history is off"));
    pane.open(&launcher);
    assert_eq!(titles(&launcher)[0], TURN_ON);
}

fn marked_blank_other_and_long_content_is_not_kept(fixture: &'static Fixture) {
    let pane = Pane::new(fixture);
    let launcher = pane.installed();
    pane.turn_on(&launcher);
    let marked = |markers: Markers| Observation {
        content: Content::Text("hunter2".into()),
        markers,
        source: Some("keepassxc.exe".into()),
    };
    assert!(pane.clipboard.change(marked(Markers {
        exclude_from_monitoring: true,
        ..Markers::default()
    })));
    pane.clipboard.change(marked(Markers {
        include_in_history: Some(false),
        ..Markers::default()
    }));
    pane.clipboard.change(marked(Markers {
        upload_to_cloud: Some(false),
        ..Markers::default()
    }));
    pane.clipboard.change(Observation {
        content: Content::Withheld,
        markers: Markers::default(),
        source: None,
    });
    pane.clipboard.change(Observation {
        content: Content::Other,
        markers: Markers::default(),
        source: None,
    });
    pane.clipboard.copy("   \n", None);
    pane.clipboard.copy(&"x".repeat(MAX_TEXT_BYTES + 1), None);
    pane.clipboard.copy("kept", None);

    assert_eq!(pane.listed(&launcher), ["kept"]);
    assert_eq!(pane.kept_on_disk(), ["kept"]);
    let file = fs::read_to_string(pane.history_path()).unwrap();
    assert!(!file.contains("hunter2"));
}

fn text_from_an_excluded_program_is_not_kept(fixture: &'static Fixture) {
    let pane = Pane::new(fixture);
    let launcher = pane.installed();
    pane.turn_on(&launcher);
    pane.open(&launcher);
    select_title(&launcher, EXCLUDE);
    block_on(launcher.activate_selected());
    assert!(launcher.view().form().is_some());
    launcher.set_field_value("program", r"C:\KeePass.exe");
    block_on(launcher.submit_form());
    assert!(matches!(launcher.view().status, Status::Error(_)));
    launcher.set_field_value("program", " KeePass.exe ");
    block_on(launcher.submit_form());
    assert_eq!(
        launcher.view().status,
        result("Text copied from KeePass.exe is not kept")
    );

    pane.clipboard.copy("secret", Some("KEEPASS.EXE"));
    pane.clipboard.copy("note", Some("notepad.exe"));
    assert_eq!(pane.listed(&launcher), ["note"]);
    assert_eq!(
        subtitle(&launcher, EXCLUDE),
        "Text copied from it is never kept · 1 excluded"
    );
    assert_eq!(
        run(&launcher, "Stop excluding keepass.exe"),
        result("Text copied from keepass.exe is kept again")
    );
    pane.clipboard.copy("secret again", Some("keepass.exe"));
    assert_eq!(pane.listed(&launcher), ["secret again", "note"]);
}

fn pausing_stops_the_watch_and_resuming_starts_it_again(fixture: &'static Fixture) {
    let pane = Pane::new(fixture);
    let launcher = pane.installed();
    pane.turn_on(&launcher);
    pane.clipboard.copy("one", None);
    pane.open(&launcher);
    assert_eq!(run(&launcher, PAUSE), result("Clipboard history is paused"));
    assert!(!pane.clipboard.watching());
    assert!(!pane.clipboard.copy("while paused", None));

    pane.open(&launcher);
    assert_eq!(
        subtitle(&launcher, RESUME),
        "Paused · 1 item kept · Nothing you copy is kept until you resume"
    );
    // Paused stays paused after a restart.
    drop(launcher);
    let launcher = pane.start();
    assert!(!pane.clipboard.watching());
    pane.open(&launcher);
    assert_eq!(
        run(&launcher, RESUME),
        result("Clipboard history is on again")
    );
    assert!(pane.clipboard.watching());
    pane.clipboard.copy("two", None);
    assert_eq!(pane.listed(&launcher), ["two", "one"]);
}

fn disabling_stops_the_watch_and_a_restart_watches_only_while_enabled(fixture: &'static Fixture) {
    let pane = Pane::new(fixture);
    let launcher = pane.installed();
    pane.turn_on(&launcher);
    pane.clipboard.copy("kept", None);
    let identity = launcher.packages()[0].identity.clone();

    block_on(launcher.set_enabled(&identity, false));
    assert!(!pane.clipboard.watching());
    assert!(!pane.clipboard.copy("while disabled", None));
    // Disabled, it stays unwatched after a restart; its history is kept.
    drop(launcher);
    let launcher = pane.start();
    assert!(!pane.clipboard.watching());
    assert_eq!(pane.kept_on_disk(), ["kept"]);

    block_on(launcher.set_enabled(&identity, true));
    assert!(pane.clipboard.watching());
    pane.clipboard.copy("after enabling", None);
    // Enabled and on, a restart watches again at once.
    drop(launcher);
    assert!(!pane.clipboard.watching());
    let launcher = pane.start();
    assert!(pane.clipboard.watching());
    pane.clipboard.copy("after restart", None);
    assert_eq!(
        pane.listed(&launcher),
        ["after restart", "after enabling", "kept"]
    );
}

fn a_slow_read_never_delays_stopping_and_is_not_kept_after_it(fixture: &'static Fixture) {
    let pane = Pane::new(fixture);
    let launcher = pane.installed();
    pane.turn_on(&launcher);
    let identity = launcher.packages()[0].identity.clone();

    // A read still in progress (a hung clipboard owner) when the package
    // is disabled: disabling does not wait for it, and it keeps nothing,
    // even once the package is enabled again and a new watch runs.
    let reading = pane.clipboard.begin_read().unwrap();
    let started = Instant::now();
    block_on(launcher.set_enabled(&identity, false));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(!pane.clipboard.watching());
    block_on(launcher.set_enabled(&identity, true));
    assert!(pane.clipboard.watching());
    reading.finish("read too late");
    assert!(pane.kept_on_disk().is_empty());

    // Pausing likewise.
    let reading = pane.clipboard.begin_read().unwrap();
    pane.open(&launcher);
    assert_eq!(run(&launcher, PAUSE), result("Clipboard history is paused"));
    reading.finish("paused meanwhile");
    assert!(pane.kept_on_disk().is_empty());
    assert!(pane.listed(&launcher).is_empty());
}

fn a_read_begun_before_clearing_is_not_kept_after_it(fixture: &'static Fixture) {
    let pane = Pane::new(fixture);
    let launcher = pane.installed();
    pane.turn_on(&launcher);
    pane.clipboard.copy("old", None);
    let reading = pane.clipboard.begin_read().unwrap();
    pane.open(&launcher);
    assert_eq!(run(&launcher, CLEAR), result("Deleted 1 kept item"));
    reading.finish("read before clearing");
    assert!(pane.kept_on_disk().is_empty());
    // A change read after clearing is kept.
    pane.clipboard.copy("after clearing", None);
    assert_eq!(pane.listed(&launcher), ["after clearing"]);
}

fn enter_copies_an_item_again_and_it_moves_to_the_front(fixture: &'static Fixture) {
    let pane = Pane::new(fixture);
    let launcher = pane.installed();
    pane.turn_on(&launcher);
    pane.clipboard.copy("first", None);
    pane.clipboard.copy("second", None);
    assert_eq!(pane.listed(&launcher), ["second", "first"]);

    assert_eq!(run(&launcher, "first"), result("Copied to the clipboard"));
    assert_eq!(pane.clipboard.written(), ["first"]);
    assert_eq!(pane.listed(&launcher), ["first", "second"]);
    assert_eq!(pane.kept_on_disk(), ["first", "second"]);
}

fn at_most_the_newest_items_are_kept_and_clear_deletes_them_all(fixture: &'static Fixture) {
    let pane = Pane::new(fixture);
    let launcher = pane.installed();
    pane.turn_on(&launcher);
    for number in 0..=MAX_ITEMS {
        pane.clipboard.copy(&format!("item {number}"), None);
    }
    let kept = pane.kept_on_disk();
    assert_eq!(kept.len(), MAX_ITEMS);
    assert_eq!(kept[0], format!("item {MAX_ITEMS}"));
    assert!(!kept.contains(&"item 0".to_string()));

    pane.open(&launcher);
    assert_eq!(
        run(&launcher, CLEAR),
        result(&format!("Deleted {MAX_ITEMS} kept items"))
    );
    assert!(pane.kept_on_disk().is_empty());
    // History is still on.
    assert!(pane.clipboard.watching());
    pane.clipboard.copy("after clearing", None);
    assert_eq!(pane.listed(&launcher), ["after clearing"]);
}

fn uninstalling_stops_the_watch_and_deletes_the_history_if_asked(fixture: &'static Fixture) {
    let pane = Pane::new(fixture);
    let launcher = pane.installed();
    pane.turn_on(&launcher);
    pane.clipboard.copy("one", None);
    pane.clipboard.copy("two", None);

    let details = pane.uninstall(&launcher, "Uninstall and delete saved data");
    assert!(
        details.contains(&"Saved data: 2 clipboard history items".to_string()),
        "{details:?}"
    );
    assert!(!pane.clipboard.watching());
    assert!(pane.kept_on_disk().is_empty());
    assert!(launcher.retained_data().is_empty());
}

fn uninstalling_and_keeping_the_history_keeps_it_for_a_reinstall(fixture: &'static Fixture) {
    let pane = Pane::new(fixture);
    let launcher = pane.installed();
    pane.turn_on(&launcher);
    pane.clipboard.copy("kept", None);

    pane.uninstall(&launcher, "Uninstall and keep saved data");
    assert!(!pane.clipboard.watching());
    assert_eq!(pane.kept_on_disk(), ["kept"]);
    assert_eq!(launcher.retained_data().len(), 1);
    launcher.back();
    select_title(&launcher, MANAGE_ROW);
    block_on(launcher.activate_selected());
    assert!(
        subtitle(
            &launcher,
            &format!("Delete retained data of {}", fixture.title)
        )
        .contains("keeps 1 clipboard history item"),
        "{:?}",
        launcher.view().rows
    );
    // Installed again from the same folder, it keeps history as it did.
    drop(launcher);
    let launcher = pane.installed();
    assert!(pane.clipboard.watching());
    assert_eq!(pane.listed(&launcher), ["kept"]);
}

fn where_the_clipboard_cannot_be_watched_history_stays_off_and_says_why(fixture: &'static Fixture) {
    let reason = "Not available: the test's clipboard cannot be watched";
    let pane = Pane::with(
        fixture,
        FakeClipboard {
            unavailable: Some(reason.into()),
            ..FakeClipboard::default()
        },
    );
    let launcher = pane.installed();
    pane.open(&launcher);
    assert!(subtitle(&launcher, TURN_ON).starts_with(&format!("{reason} · Off")));
    assert_eq!(
        run(&launcher, TURN_ON),
        Status::Error(format!("The extension reported an error: {reason}"))
    );
    assert_eq!(pane.clipboard.started(), 0);
    assert!(pane.history_file().is_none());
}

fn without_a_clipboard_pane_keeps_nothing_and_says_so(fixture: &'static Fixture) {
    let pane = Pane::new(fixture);
    let launcher = Launcher::with_packages(
        Runtime::start(),
        vec![],
        pane.data.path().join("extensions"),
    );
    install(&launcher, pane.folder());
    pane.open(&launcher);
    assert!(
        subtitle(&launcher, TURN_ON)
            .starts_with("Not available: this Pane does not watch the clipboard · Off"),
        "{:?}",
        launcher.view().rows
    );
    assert_eq!(
        run(&launcher, TURN_ON),
        Status::Error(
            "The extension reported an error: Not available: this Pane does not watch the \
             clipboard"
                .into()
        )
    );
}

fn the_package_is_offered_only_on_windows_so_far(fixture: &'static Fixture) {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    copy_package(fixture, source.path(), false);
    let launcher =
        Launcher::with_packages(Runtime::start(), vec![], data.path().join("extensions"))
            .with_clipboard(pane_core::clipboard::none());
    install(&launcher, source.path());
    block_on(launcher.set_query("clipboard"));
    let row = launcher
        .view()
        .rows
        .into_iter()
        .find(|row| row.title == fixture.command)
        .expect("the command is listed");
    if platforms::this_system() == pane_core::Platform::Windows {
        assert_eq!(row.unavailable, None);
    } else {
        assert_eq!(
            row.unavailable,
            Some(Unavailable::OnThisSystem(platforms::only(
                "this command",
                "Windows"
            )))
        );
    }
}

/// Declares one test per check for each language's clipboard package.
macro_rules! contract {
    ($($check:ident),* $(,)?) => {
        mod rust {
            $(#[test] fn $check() { super::$check(&super::RUST) })*
        }
        mod javascript {
            $(#[test] fn $check() { super::$check(&super::JAVASCRIPT) })*
        }
        mod typescript {
            $(#[test] fn $check() { super::$check(&super::TYPESCRIPT) })*
        }
    };
}

contract!(
    nothing_is_watched_or_kept_until_history_is_turned_on,
    turning_history_off_stops_the_watch_and_keeps_the_items,
    marked_blank_other_and_long_content_is_not_kept,
    text_from_an_excluded_program_is_not_kept,
    pausing_stops_the_watch_and_resuming_starts_it_again,
    disabling_stops_the_watch_and_a_restart_watches_only_while_enabled,
    a_slow_read_never_delays_stopping_and_is_not_kept_after_it,
    a_read_begun_before_clearing_is_not_kept_after_it,
    enter_copies_an_item_again_and_it_moves_to_the_front,
    at_most_the_newest_items_are_kept_and_clear_deletes_them_all,
    uninstalling_stops_the_watch_and_deletes_the_history_if_asked,
    uninstalling_and_keeping_the_history_keeps_it_for_a_reinstall,
    where_the_clipboard_cannot_be_watched_history_stays_off_and_says_why,
    without_a_clipboard_pane_keeps_nothing_and_says_so,
    the_package_is_offered_only_on_windows_so_far,
);
