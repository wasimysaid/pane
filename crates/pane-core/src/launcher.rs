//! The launcher model: the public host interface driven by the native window
//! and by tests alike.
//!
//! Every user action is a method on [`Launcher`]; [`Launcher::view`] returns a
//! snapshot of what the window should show. Actions that call into an
//! extension update the snapshot immediately (for example to "running") and
//! return a future that applies the extension's reply when awaited.
//!
//! Every reply is checked against the screen it was requested from: once the
//! user has left that screen, the reply is discarded, and a custom view that
//! opened after the user left is closed again.
//!
//! A call into an installed package also belongs to the package's
//! generation current when the user asked for it (see `generation`):
//! disabling, reloading or updating the package ends it, which stops the call
//! in the runtime, and its answer is never shown, even on a screen that is
//! still current. Leaving a screen only discards its replies; it does not
//! stop the call. So does pausing a package that keeps failing (see
//! `pausing`). The one exception is root search's own calls for results
//! computed from the query: a search owns them, so a newer query, or
//! leaving root search, cancels those still pending.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};

mod aliases;
mod background;
mod choices;
mod command_search;
mod hotkeys;
mod indexed;
mod network;

use crate::changes::ChangeSender;
use crate::dependencies;
use crate::extension_data::{ExtensionData, PackageData};
use crate::files::FileAccess;
use crate::generation::End;
use crate::helpers;
use crate::hotkeys::{self as system_hotkeys, Hotkeys};
use crate::links::{self, LinkOpener, NoOpener};
use crate::operations::{self, Installed};
use crate::packages::{
    InstalledPackage, PackageError, PackageIdentity, PauseCause, RetainedData, SavedData,
    SourcePackage, Store, paused_reason,
};
use crate::platform::{self, Platform};
use crate::runtime::{
    CallError, CustomViewInfo, CustomViewRole, FieldKind, FieldValue, Form, Frame, Item, Point,
    ResultListing, RootAction, RootResult as ComputedResult, Runtime, ViewEvent, ViewId,
    WeakRuntime,
};
use crate::search::{self, Keys, Query};

mod dependents;
mod developing;
mod files;
mod install;
mod pausing;
mod recovery;
mod reload;
mod retained;
mod scheduled;
mod uninstall;

use aliases::AliasChoices;
use background::Background;
pub use background::BackgroundChanges;
use choices::Record;
use developing::Developing;
pub use developing::{BuildFailure, Development};
use hotkeys::Bindings;
use pausing::{Pauses, Recorder};
pub use scheduled::{ScheduledTask, TaskOutcome};

/// The id of the root row that installs a package from a local folder.
const INSTALL_FROM_FOLDER: &str = "pane.install-from-folder";

/// The folder beside the managed copies where packages downloaded from npm
/// are unpacked until they are installed.
const DOWNLOADS_DIR: &str = "downloads";

/// The id of the root row that installs a package from npm.
const INSTALL_FROM_NPM: &str = "pane.install-from-npm";

/// The id of the npm package field of Pane's own form that asks which npm
/// package to install.
const NPM_PACKAGE_FIELD: &str = "package";

/// The id of the root row that lists installed packages to enable or
/// disable them.
const MANAGE_EXTENSIONS: &str = "pane.manage-extensions";

/// A command offered in root search, backed by one extension component.
#[derive(Clone, Debug)]
pub struct CommandRegistration {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub component: PathBuf,
    /// Whether the command takes a query (`"takesQuery": true`): text typed
    /// into root search, sent to it through its alias or as a fallback.
    pub takes_query: bool,
    /// Whether the command searches as the user types into its own search
    /// field once it is open (`"search": true`); root search never asks it.
    pub search: bool,
}

impl CommandRegistration {
    /// The command's id in its package manifest: the part of [`Self::id`]
    /// after the package identity's key, for an installed command.
    pub fn manifest_id(&self) -> &str {
        choices::split(&self.id).1
    }
}

/// Which screen the launcher shows, with what only that screen has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Root search: the root results matching `query`, the text typed into
    /// it, best match first, or every root result when it is empty.
    Root { query: String },
    /// An opened command's list view.
    Command,
    /// An opened command that searches as the user types into its own
    /// search field, holding `query`, the text typed there: while it is
    /// blank, the command's list view; otherwise what the command found
    /// for it. Only the opened command is asked, never root search's
    /// providers.
    CommandSearch { query: String },
    /// A package folder's identity and compatibility, before installing it,
    /// as lines of information under the title.
    Package { details: Vec<String> },
    /// A form opened from an item of the command's list view. It has no
    /// rows.
    Form(FormView),
    /// The installed packages, each enabled or disabled, with lines of
    /// information under the title.
    Extensions { details: Vec<String> },
    /// A custom view opened from an item of the command's list view. It has
    /// no rows.
    CustomView(CustomViewSnapshot),
    /// What an installed package that uses the network did on it this
    /// session (the addresses it tried to reach), as lines of information
    /// under the title. It has no rows.
    NetworkDetails {
        identity: PackageIdentity,
        details: Vec<String>,
    },
    /// Why Pane paused an installed package, as lines of information under
    /// the title, with a row that retries it.
    PauseDetails {
        identity: PackageIdentity,
        details: Vec<String>,
    },
    /// Why the last development build of an installed package failed, as
    /// lines of information under the title (its command and output), with
    /// a row that builds it again.
    BuildDetails {
        identity: PackageIdentity,
        details: Vec<String>,
    },
    /// `question` about an installed package before Pane acts on it, with
    /// lines of information under the title, answered by choosing a row.
    Confirm {
        question: Question,
        details: Vec<String>,
    },
    /// Why Pane's extension runtime stopped after its thread crashed, and
    /// what Pane did, as lines of information under the title, with a row
    /// that restarts it when Pane did not.
    RuntimeDetails { details: Vec<String> },
    /// Asks for the keys of a global hotkey that opens the installed command
    /// with id `command` from any application, with lines of information
    /// under the title. The window sends the keys pressed to
    /// [`Launcher::record_hotkey`]; the rows offer to remove its hotkey.
    Hotkey {
        command: String,
        details: Vec<String>,
    },
}

/// What a confirmation screen asks before Pane acts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Question {
    /// Whether to clear the cache of the installed package with this
    /// identity.
    ClearCache(PackageIdentity),
    /// Whether to uninstall the installed package with this identity, and
    /// whether to keep its saved data.
    Uninstall(PackageIdentity),
    /// Whether to delete the retained data of this identity, which is not
    /// installed.
    DeleteRetained(PackageIdentity),
    /// Whether to disable the installed package with this identity together
    /// with the enabled packages that require it.
    DisableDependents(PackageIdentity),
    /// Whether to uninstall the installed package with this identity
    /// together with the installed packages that require it, and whether to
    /// keep their saved data.
    UninstallDependents(PackageIdentity),
}

/// A selectable row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    /// Why the row's action cannot be used; `None` when it can. An
    /// unavailable row stays listed and selectable, and activating it shows
    /// the reason instead of calling the extension.
    pub unavailable: Option<Unavailable>,
}

/// Why a row's action cannot be used, with the reason to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// It does not work on this system: the command, action or package does
    /// not support it, or the system lacks what it needs (such as global
    /// hotkeys).
    OnThisSystem(String),
    /// Its package is paused after an error until the user retries it.
    Paused(String),
}

impl Row {
    /// The row of a result a command answered with (computed or indexed
    /// root results, search results), available. Its id is the result's,
    /// under `scope` if given (`<scope>:<id>`), so results of different
    /// commands among root search's rows cannot share one.
    fn listed(listing: ResultListing, scope: Option<&str>) -> Row {
        Row {
            id: match scope {
                Some(scope) => format!("{scope}:{}", listing.id),
                None => listing.id,
            },
            title: listing.title,
            subtitle: listing.subtitle,
            unavailable: None,
        }
    }
}

/// An opened command's own list: its rows and what activating each does.
#[derive(Clone, Default)]
struct CommandList {
    rows: Vec<Row>,
    entries: Vec<Entry>,
}

impl CommandList {
    /// The list of a command whose list view has `items`.
    fn of(items: Vec<Item>) -> CommandList {
        let (rows, entries) = items
            .into_iter()
            .map(|item| {
                let unavailable = platform::unavailable(item.platforms.as_deref(), "this action");
                let entry = match (&unavailable, item.form, item.custom_view) {
                    (Some(reason), _, _) => Entry::Unavailable(reason.clone()),
                    (None, Some(form), _) => Entry::Form(item.id.clone(), form),
                    (None, None, Some(info)) => Entry::CustomView(item.id.clone(), info),
                    (None, None, None) => Entry::Run(item.id.clone()),
                };
                let row = Row {
                    id: item.id,
                    title: item.title,
                    subtitle: item.subtitle,
                    unavailable: unavailable.map(Unavailable::OnThisSystem),
                };
                (row, entry)
            })
            .unzip();
        CommandList { rows, entries }
    }
}

impl Unavailable {
    /// The reason, as shown to the user.
    pub fn reason(&self) -> &str {
        match self {
            Unavailable::OnThisSystem(reason) | Unavailable::Paused(reason) => reason,
        }
    }
}

/// Feedback about the most recent action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    /// An extension call or package operation is in progress.
    Running,
    /// Work Pane does in the background is in progress, saying what, such
    /// as building a package being developed.
    Progress(String),
    /// The outcome of the most recent action, such as the extension's answer.
    Result(String),
    /// Why the most recent action failed. For a rejected form field this is
    /// "<field label>: <message>".
    Error(String),
}

/// An open form, as the user is filling it in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormView {
    pub fields: Vec<FormField>,
    pub submit_label: String,
}

/// One field of an open form with its current value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormField {
    pub id: String,
    pub label: String,
    pub kind: FieldKind,
    /// The text of a text field, or the id of the chosen option.
    pub value: String,
    /// Why the extension rejected this field on the last submission.
    pub error: Option<String>,
}

/// An open custom view as the extension last drew it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomViewSnapshot {
    /// Which opened view this is: a view opened again, even of the same
    /// item, has another id.
    pub id: ViewId,
    /// Names the view to assistive technology.
    pub label: String,
    pub role: CustomViewRole,
    /// The latest drawing: the answer to the most recent event whose answer
    /// has arrived, or the first drawing.
    pub frame: Frame,
}

/// A snapshot of what the launcher shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LauncherView {
    pub screen: Screen,
    pub title: String,
    /// Empty on the form and custom view screens.
    pub rows: Vec<Row>,
    /// Index into `rows`; `None` when there are no rows.
    pub selected: Option<usize>,
    pub status: Status,
}

impl LauncherView {
    /// `screen` titled `title`, with no rows and an idle status.
    fn new(screen: Screen, title: impl Into<String>) -> LauncherView {
        LauncherView {
            screen,
            title: title.into(),
            rows: Vec::new(),
            selected: None,
            status: Status::Idle,
        }
    }

    /// This view listing `rows`, with the first selected.
    fn with_rows(self, rows: Vec<Row>) -> LauncherView {
        LauncherView {
            selected: first_index(&rows),
            rows,
            ..self
        }
    }

    /// The text typed into root search; `None` on other screens.
    pub fn query(&self) -> Option<&str> {
        match &self.screen {
            Screen::Root { query } => Some(query),
            _ => None,
        }
    }

    /// The text of the search field on screen: root search's query, or the
    /// search of an open command that searches as the user types; `None` on
    /// screens without one.
    pub fn search_field(&self) -> Option<&str> {
        match &self.screen {
            Screen::Root { query } | Screen::CommandSearch { query } => Some(query),
            _ => None,
        }
    }

    /// Lines of information under the title, such as a package's source and
    /// compatibility; empty on screens without any.
    pub fn details(&self) -> &[String] {
        match &self.screen {
            Screen::Package { details }
            | Screen::Extensions { details }
            | Screen::Confirm { details, .. }
            | Screen::PauseDetails { details, .. }
            | Screen::NetworkDetails { details, .. }
            | Screen::BuildDetails { details, .. }
            | Screen::RuntimeDetails { details }
            | Screen::Hotkey { details, .. } => details,
            _ => &[],
        }
    }

    /// The open form, on the form screen.
    pub fn form(&self) -> Option<&FormView> {
        match &self.screen {
            Screen::Form(form) => Some(form),
            _ => None,
        }
    }

    /// The open custom view, on the custom view screen.
    pub fn custom_view(&self) -> Option<&CustomViewSnapshot> {
        match &self.screen {
            Screen::CustomView(view) => Some(view),
            _ => None,
        }
    }
}

/// The launcher. Cloning shares the same state.
#[derive(Clone)]
pub struct Launcher {
    runtime: Result<Runtime, CallError>,
    /// Commands supplied by this build rather than by installed packages.
    commands: Arc<[CommandRegistration]>,
    /// Where installed packages are kept, when installing packages is on.
    installation: Option<Installation>,
    /// Opens the web links of computed results.
    links: Arc<dyn LinkOpener>,
    /// Registers the global hotkeys the user assigns with the system.
    hotkeys: Arc<dyn Hotkeys>,
    /// Reads packages from folders and downloads them from npm.
    sources: install::Sources,
    /// The packages being developed: built and reloaded on save.
    developing: Arc<Developing>,
    /// Tells the window that the launcher changed in the background, such
    /// as after a runtime crash; given with development
    /// ([`Launcher::with_development`]).
    changes: Option<ChangeSender>,
    /// Runs the installed packages' background work, once made to
    /// ([`Launcher::with_background`]).
    background: Option<Arc<Background>>,
    state: Arc<Mutex<State>>,
}

/// A launcher that does not keep itself or its runtime running, for the
/// runtime to hold.
struct WeakLauncher {
    runtime: Result<WeakRuntime, CallError>,
    commands: Arc<[CommandRegistration]>,
    installation: Option<Installation>,
    links: Arc<dyn LinkOpener>,
    hotkeys: Arc<dyn Hotkeys>,
    sources: install::Sources,
    developing: std::sync::Weak<Developing>,
    changes: Option<ChangeSender>,
    background: Option<std::sync::Weak<Background>>,
    state: std::sync::Weak<Mutex<State>>,
}

impl WeakLauncher {
    /// The launcher, unless it or its runtime has stopped.
    fn upgrade(&self) -> Option<Launcher> {
        let runtime = match &self.runtime {
            Ok(runtime) => Ok(runtime.upgrade()?),
            Err(error) => Err(error.clone()),
        };
        Some(Launcher {
            runtime,
            commands: self.commands.clone(),
            installation: self.installation.clone(),
            links: self.links.clone(),
            hotkeys: self.hotkeys.clone(),
            sources: self.sources.clone(),
            developing: self.developing.upgrade()?,
            changes: self.changes.clone(),
            background: match &self.background {
                Some(background) => Some(background.upgrade()?),
                None => None,
            },
            state: self.state.upgrade()?,
        })
    }
}

/// Pane's managed package location and the installed packages' extension
/// data,
/// kept beside it.
#[derive(Clone)]
struct Installation {
    store: Arc<Mutex<Store>>,
    data: ExtensionData,
    /// Where they are kept, with Pane's other records such as the hotkeys.
    dir: PathBuf,
    /// Writes which packages are paused, in the background.
    records: Recorder,
}

struct State {
    view: LauncherView,
    /// What activating each row of the current screen does.
    entries: Vec<Entry>,
    /// Every root result in root search order, built when root search is
    /// shown or refreshed, so that searching only ranks them.
    root: Vec<RootResult>,
    /// The root results commands computed from the current query, listed
    /// first; each command's results are added when it answers.
    computed: Vec<Computed>,
    /// The root results commands supplied ahead of the query, such as the
    /// installed applications, listed for a query that is not blank.
    indexes: indexed::Indexes,
    /// Incremented on every search, so that an answer arriving for an
    /// earlier search, even of the same query, is discarded.
    search_epoch: u64,
    /// Kept while the current search's calls for computed results may run:
    /// dropping it, when the query changes or root search is left, cancels
    /// those still pending (see [`State::next_screen`]).
    search_alive: Option<tokio::sync::oneshot::Sender<()>>,
    /// The folders granted to packages and their listings, shared with the
    /// runtime; `None` without a runtime.
    files: Option<FileAccess>,
    /// The component of the command whose view is open.
    open: Option<PathBuf>,
    /// The open command's search, when it searches as the user types.
    searching: Option<command_search::Searching>,
    /// The form on screen, if one is open.
    form: Option<OpenForm>,
    /// The custom view on screen, if one is open.
    custom_view: Option<OpenCustomView>,
    /// Incremented on every navigation, so a reply that arrives after the
    /// user has left the screen it was requested from is discarded.
    screen_epoch: u64,
    /// The installed packages. Whether each is enabled here is the user's
    /// latest choice, which applies at once, even while it is still being
    /// recorded.
    packages: Vec<InstalledPackage>,
    /// The identities that are not installed but whose data Pane keeps, as
    /// last recorded in the store.
    retained: Vec<RetainedData>,
    /// Packages being enabled or disabled, reloaded or updated, with which;
    /// another change to one of them is refused meanwhile (see
    /// [`State::claim`]).
    changing: HashMap<PackageIdentity, Changing>,
    /// Why the installed packages could not be read, if they could not.
    store_problem: Option<String>,
    /// The packages Pane paused after they failed, each with why, and the
    /// crashes counted towards pausing a package (see `pausing`).
    paused: Pauses,
    /// The global hotkeys the user assigned to commands.
    bindings: Bindings,
    /// The aliases and fallbacks the user gave commands.
    aliases: Record<AliasChoices>,
    /// The scheduled tasks the user turned on, and their runs.
    tasks: scheduled::Tasks,
    /// Wakes the background work's driver when a change to a package ends
    /// ([`State::release`]); none until the launcher runs background work.
    background: Option<std::sync::Weak<Background>>,
    /// The query root search showed when the status line began showing a
    /// command's answer to a query sent from it (or its sending), so that
    /// changing the query clears it.
    sent_from: Option<String>,
    /// The newest status of a developed package's builds, kept while
    /// another screen is shown (see `developing`).
    development_status: Option<(PackageIdentity, Status)>,
}

/// What is happening to a package, which stops another change to it
/// meanwhile.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Changing {
    /// Its enabling or disabling has taken effect and is being recorded.
    Recording,
    /// It is being reloaded, or started again after it failed to start.
    Reloading,
    /// Its managed copy is being replaced from a package folder.
    Updating,
    /// It is being uninstalled.
    Uninstalling,
    /// It is not installed, and its retained data is being deleted.
    DeletingRetained,
    /// It is being installed with another package, or an install relies on
    /// it as a required dependency staying as it is.
    Installing,
}

impl Changing {
    /// What is happening, after the package's title: "is reloading".
    fn doing(self) -> &'static str {
        match self {
            Changing::Recording => "is being enabled or disabled",
            Changing::Reloading => "is reloading",
            Changing::Updating => "is updating",
            Changing::Uninstalling => "is being uninstalled",
            Changing::DeletingRetained => "is having its retained data deleted",
            Changing::Installing => "is part of an install in progress",
        }
    }
}

impl State {
    /// The installed package with `identity`.
    fn package(&self, identity: &PackageIdentity) -> Option<&InstalledPackage> {
        self.packages
            .iter()
            .find(|package| package.identity == *identity)
    }

    /// Whether `package`'s code may run: it is enabled and not paused.
    fn runs(&self, package: &InstalledPackage) -> bool {
        package.enabled && !self.paused.is_paused(&package.identity)
    }

    /// The title of the installed package with `identity`, the title its
    /// retained data was kept under if it is not installed, or else its
    /// identity.
    fn title_of(&self, identity: &PackageIdentity) -> String {
        self.package(identity)
            .map(InstalledPackage::title)
            .or_else(|| {
                self.retained
                    .iter()
                    .find(|retained| retained.identity == *identity)
                    .map(|retained| retained.title.clone())
            })
            .unwrap_or_else(|| identity.to_string())
    }

    /// Notes that `what` begins on the package with `identity`, unless
    /// something else is happening to it: then it says what, and returns
    /// false. A second enabling or disabling while one is recorded is ignored
    /// without a word, as pressing Enter twice would do.
    fn claim(&mut self, identity: &PackageIdentity, what: Changing) -> bool {
        let busy = match self.changing.get(identity) {
            None => {
                self.changing.insert(identity.clone(), what);
                return true;
            }
            Some(Changing::Recording) => return false,
            Some(busy) => busy.doing(),
        };
        self.view.status = Status::Error(format!("{} {busy}", self.title_of(identity)));
        false
    }

    /// Notes that each `(identity, what)` of `claims` begins, all of them or
    /// none: if something is already happening to one of them, claims
    /// nothing and returns that identity with what is happening to it.
    fn claim_all(
        &mut self,
        claims: &[(PackageIdentity, Changing)],
    ) -> Result<(), (PackageIdentity, Changing)> {
        for (identity, _) in claims {
            if let Some(&busy) = self.changing.get(identity) {
                return Err((identity.clone(), busy));
            }
        }
        for (identity, what) in claims {
            self.changing.insert(identity.clone(), *what);
        }
        Ok(())
    }

    /// Notes that what began with [`State::claim`] on the package with
    /// `identity` has ended.
    fn release(&mut self, identity: &PackageIdentity) {
        self.changing.remove(identity);
        // Its background work may run again (see `background`).
        if let Some(background) = self.background.as_ref().and_then(std::sync::Weak::upgrade) {
            background.wake();
            background.changed();
        }
    }

    /// Notes that the user left the screen on display: replies for it are
    /// discarded from now on, and root search's pending calls for computed
    /// results are cancelled.
    fn next_screen(&mut self) {
        self.screen_epoch += 1;
        self.search_alive = None;
        // A granted folder is listed again on the next visit, and a
        // listing being made for this one stops.
        if let Some(files) = &self.files {
            files.new_visit();
        }
    }
}

/// An enabling or disabling that has taken effect and is being recorded:
/// of one package, or of a package and the packages that require it, the
/// package asked about first.
struct Change {
    identities: Vec<PackageIdentity>,
    enabled: bool,
}

/// What the launcher keeps about the open form besides its view.
struct OpenForm {
    /// What submitting the form does.
    purpose: FormPurpose,
    /// The command view that Back returns to.
    return_to: LauncherView,
    /// Whether a submission is waiting for the extension's reply; further
    /// submissions are ignored meanwhile.
    submitting: bool,
}

/// What submitting a form does.
enum FormPurpose {
    /// Sends it to the open command, for its item with this id.
    Item(String),
    /// Sets the alias of the installed command with this id (Pane's own).
    Alias(String),
    /// Previews the npm package it names (Pane's own).
    Npm,
}

/// What the launcher keeps about the open custom view besides its snapshot.
struct OpenCustomView {
    /// The view in the runtime; closed when the view leaves the screen.
    id: ViewId,
    /// The command view that Back returns to.
    return_to: LauncherView,
    /// Whether the primary pointer button was pressed over the view and is
    /// still held; pointer moves and the release are sent only meanwhile.
    pressed: bool,
    /// How many events were sent to the view.
    sent: u64,
    /// The number of the event whose answer is on screen, so an older answer
    /// arriving late does not replace a newer one.
    shown: u64,
    /// Pointer moves sent to the view and not answered yet. While there are
    /// any, a further move waits in `waiting_move` instead of being sent.
    moves_in_flight: u32,
    /// The latest move of a drag that has not been sent: sent when the
    /// moves in flight are answered, or before the next other event.
    waiting_move: Option<Point>,
}

/// An event sent to the open view, whose answer is still to be shown.
struct SentEvent {
    /// The event's number among those sent to the view.
    number: u64,
    is_move: bool,
    reply: Pin<Box<dyn Future<Output = Result<Frame, CallError>> + Send>>,
}

impl OpenCustomView {
    fn send(&mut self, runtime: &Runtime, event: ViewEvent) -> SentEvent {
        self.sent += 1;
        let is_move = matches!(event, ViewEvent::PointerMove(_));
        if is_move {
            self.moves_in_flight += 1;
        }
        SentEvent {
            number: self.sent,
            is_move,
            reply: Box::pin(runtime.view_event(self.id, event)),
        }
    }
}

/// A result root search can list: its row, what activating it does, and
/// its text as the query is matched against it.
struct RootResult {
    row: Row,
    entry: Entry,
    keys: Keys,
    /// The installed command it opens, for its alias and fallback.
    target: Option<aliases::Target>,
}

/// A root result a command computed from the current query.
struct Computed {
    /// The component of the command that computed it.
    component: PathBuf,
    row: Row,
    entry: Entry,
}

/// What activating a row does.
#[derive(Clone)]
enum Entry {
    /// Copy this text to the clipboard, which the window does (root).
    Copy(String),
    /// Open this web address with the link opener (root).
    OpenUrl(String),
    /// Open the file with id `id` in the latest listing of the package with
    /// identity key `owner`, named `name` (root).
    OpenFile {
        owner: String,
        id: String,
        name: String,
    },
    /// Nothing in the launcher: the window asks for the folder to grant
    /// this package, then calls [`Launcher::grant_folder`] (command view).
    ChooseFolder(PackageIdentity),
    /// Take back the folder granted to this package (command view).
    StopSharingFolder(PackageIdentity),
    /// Open the installed application `id`, named `name` (root).
    OpenApplication { id: String, name: String },
    /// Open this command (root).
    Open(Opening),
    /// Send a query to a command that takes one, and show its answer
    /// (root: an alias or fallback).
    Send(aliases::Sending),
    /// Explain why this installed package cannot load (root).
    Broken(String),
    /// Explain why this command (root) or this item's action (command view)
    /// is unavailable on this system, or paused; the extension is not called.
    Unavailable(String),
    /// Nothing in the launcher: the window asks for a folder (root).
    InstallFromFolder,
    /// Ask which npm package to install (root).
    AskNpm,
    /// Run the open command's item with this id.
    Run(String),
    /// Open this form of the open command's item with this id.
    Form(String, Form),
    /// Open the custom view of the open command's item with this id.
    CustomView(String, CustomViewInfo),
    /// Install the previewed package from this folder or npm package, or
    /// replace its installed copy, as the preview's plan assumed things to
    /// be.
    Install(install::Request, Mode, dependencies::Assumptions),
    /// Show the installed packages (root).
    Manage,
    /// Enable this installed package if it is disabled, else disable it, or
    /// first ask about the enabled packages that require it.
    Toggle(PackageIdentity),
    /// Disable this installed package and the packages that require it,
    /// which the confirmation showed (confirmation).
    DisableAll(PackageIdentity, Vec<PackageIdentity>),
    /// Reload this installed package from its source folder.
    Reload(PackageIdentity),
    /// Start again this package, which Pane paused after it failed.
    Retry(PackageIdentity),
    /// Show why Pane paused this package (extension list).
    PauseDetails(PackageIdentity),
    /// Show what this package did on the network this session (extension
    /// list).
    NetworkDetails(PackageIdentity),
    /// Show why Pane's extension runtime stopped (extension list).
    RuntimeDetails,
    /// Start Pane's extension runtime again after it crashed and Pane did
    /// not restart it (extension list, runtime details).
    RestartRuntime,
    /// Build and reload this package after each save in its source folder
    /// (extension list).
    Develop(PackageIdentity),
    /// Stop developing this package (extension list).
    StopDeveloping(PackageIdentity),
    /// Show why this developed package's last build failed (extension
    /// list).
    BuildDetails(PackageIdentity),
    /// Build this developed package now (build details).
    BuildAgain(PackageIdentity),
    /// Ask whether to clear this installed package's cache (extension list).
    AskClearCache(PackageIdentity),
    /// Clear this installed package's cache (confirmation).
    ClearCache(PackageIdentity),
    /// Ask for the keys of the hotkey of the command with this id
    /// (extension list).
    AskHotkey(String),
    /// Remove the hotkey of the command with this id (hotkey screen).
    RemoveHotkey(String),
    /// Show the form setting the alias of the command with this id
    /// (extension list).
    AskAlias(String),
    /// Make the command with this id a fallback, or no longer one
    /// (extension list).
    ToggleFallback(String),
    /// Forget the alias and fallback of the command with this id, which
    /// cannot be listed (extension list).
    ForgetChoices(String),
    /// Turn the schedule of the scheduled task of the command with this id
    /// on, or off (extension list).
    ToggleTask(String),
    /// Ask whether to uninstall this installed package, and whether to keep
    /// its saved data (extension list).
    AskUninstall(PackageIdentity),
    /// Uninstall this installed package, keeping or deleting its saved data
    /// (confirmation).
    Uninstall(PackageIdentity, SavedData),
    /// Uninstall this installed package and the packages that require it,
    /// which the confirmation showed, keeping or deleting their saved data
    /// (confirmation).
    UninstallAll(PackageIdentity, Vec<PackageIdentity>, SavedData),
    /// Ask whether to delete the retained data of this identity, which is
    /// not installed (extension list).
    AskDeleteRetained(PackageIdentity),
    /// Delete the retained data of this identity (confirmation).
    DeleteRetained(PackageIdentity),
    /// Return to the extension list without acting (confirmation).
    Cancel,
}

/// A command root search can open.
#[derive(Clone)]
struct Opening {
    component: PathBuf,
    /// Its id in its package manifest, sent with each of its searches.
    command: String,
    /// Whether it searches as the user types into its own search field
    /// ([`CommandRegistration::search`]).
    search: bool,
}

impl Opening {
    fn of(command: &CommandRegistration) -> Opening {
        Opening {
            component: command.component.clone(),
            command: command.manifest_id().to_owned(),
            search: command.search,
        }
    }
}

#[derive(Clone)]
enum Mode {
    Install,
    /// Replace the managed copy of the installed package with this identity.
    Update(PackageIdentity),
}

impl Launcher {
    /// Creates a launcher at root search. A runtime that failed to start
    /// leaves navigation usable and explains the failure when a command opens.
    pub fn new(runtime: Result<Runtime, CallError>, commands: Vec<CommandRegistration>) -> Self {
        Launcher::create(runtime, commands, None)
    }

    /// Creates a launcher that also offers the commands of the packages
    /// installed in `packages_dir` and installs packages there. Listing them
    /// reads only their manifests: no guest runs until a command opens.
    pub fn with_packages(
        runtime: Result<Runtime, CallError>,
        commands: Vec<CommandRegistration>,
        packages_dir: PathBuf,
    ) -> Self {
        let store = Arc::new(Mutex::new(Store::open(packages_dir.clone())));
        // Only an install in progress needs what it downloaded.
        crate::npm::remove_abandoned_downloads(
            &packages_dir.join(DOWNLOADS_DIR),
            std::time::SystemTime::now(),
        );
        let installation = Installation {
            data: ExtensionData::open(&packages_dir),
            dir: packages_dir,
            records: Recorder::start(store.clone()),
            store,
        };
        Launcher::create(runtime, commands, Some(installation))
    }

    fn create(
        runtime: Result<Runtime, CallError>,
        commands: Vec<CommandRegistration>,
        installation: Option<Installation>,
    ) -> Self {
        let (packages, retained, store_problem, paused, bindings, aliases) = match &installation {
            Some(installation) => {
                let store = installation.store.lock().unwrap_or_else(|p| p.into_inner());
                let bindings = Bindings::open(&installation.dir);
                let aliases = Record::open(&installation.dir);
                (
                    store.installed(),
                    store.retained(),
                    store.problem(),
                    store.paused(),
                    bindings,
                    aliases,
                )
            }
            None => (
                Vec::new(),
                Vec::new(),
                None,
                Vec::new(),
                Bindings::default(),
                Record::default(),
            ),
        };
        let mut state = State {
            // Replaced by root search below.
            view: LauncherView::new(Screen::Command, ""),
            entries: Vec::new(),
            root: Vec::new(),
            computed: Vec::new(),
            indexes: indexed::Indexes::default(),
            search_epoch: 0,
            search_alive: None,
            files: runtime.as_ref().ok().map(Runtime::file_access),
            open: None,
            searching: None,
            form: None,
            custom_view: None,
            screen_epoch: 0,
            packages,
            retained,
            changing: HashMap::new(),
            development_status: None,
            store_problem,
            paused: Pauses::default(),
            bindings,
            aliases,
            tasks: installation
                .as_ref()
                .map_or_else(scheduled::Tasks::default, |installation| {
                    scheduled::Tasks::open(&installation.dir)
                }),
            background: None,
            sent_from: None,
        };
        if let (Some(installation), Some(files)) = (&installation, &state.files) {
            files.open_record(&installation.dir);
        }
        if let Some(installation) = &installation {
            for package in &state.packages {
                installation
                    .data
                    .set_enabled(&package.identity, package.enabled);
            }
            // A package paused before Pane stopped stays paused, if its
            // code is still the one that failed.
            for (identity, pause) in paused {
                let same = state
                    .package(&identity)
                    .is_some_and(|p| p.enabled && p.version() == pause.version);
                if same {
                    installation.data.pause(&identity);
                    state.paused.restore(identity, pause);
                }
            }
        }
        let sources = install::Sources {
            registry: crate::npm::Registry::npmjs(),
            downloads: installation.as_ref().map(|i| i.dir.join(DOWNLOADS_DIR)),
        };
        let launcher = Launcher {
            runtime,
            commands: commands.into(),
            installation,
            links: Arc::new(NoOpener),
            hotkeys: system_hotkeys::none(),
            sources,
            developing: Arc::new(Developing::new(None, None)),
            changes: None,
            background: None,
            state: Arc::new(Mutex::new(state)),
        };
        if let (Ok(runtime), Some(installation)) = (&launcher.runtime, &launcher.installation) {
            // Operation calls see the packages as the launcher has them.
            let state = Arc::downgrade(&launcher.state);
            let data = installation.data.clone();
            runtime.set_directory(Arc::new(move || Installed {
                packages: state.upgrade().map_or_else(Vec::new, |state| {
                    let state = state.lock().unwrap_or_else(|p| p.into_inner());
                    state.packages.clone()
                }),
                data: Some(data.clone()),
            }));
        }
        launcher.report_failures();
        launcher.show_root(&mut launcher.lock(), None);
        launcher
    }

    /// This launcher opening web links, such as quicklinks, with `links`,
    /// normally the system's handler. Without one, opening a link explains
    /// that this Pane has no link handler.
    pub fn with_link_opener(self, links: Arc<dyn LinkOpener>) -> Self {
        let launcher = Launcher { links, ..self };
        launcher.report_failures();
        launcher
    }

    /// This launcher downloading npm packages from `registry` rather than
    /// from the public npm registry: one on this computer, for tests and
    /// development ([`crate::npm::Registry::local`]). Release builds have
    /// no way to replace the public registry.
    #[cfg(any(test, debug_assertions))]
    pub fn with_npm_registry(self, registry: crate::npm::Registry) -> Self {
        let sources = install::Sources {
            registry,
            ..self.sources.clone()
        };
        Launcher { sources, ..self }
    }

    /// This launcher registering the global hotkeys the user assigns with
    /// `hotkeys`, normally the system's ([`crate::hotkeys::native`]); the
    /// recorded ones are registered now. Without it, assigning a hotkey
    /// explains that this Pane has none.
    pub fn with_hotkeys(self, hotkeys: Arc<dyn Hotkeys>) -> Self {
        let launcher = Launcher { hotkeys, ..self };
        launcher.sync_hotkeys(&mut launcher.lock());
        launcher.report_failures();
        launcher
    }

    /// Has the runtime tell this launcher of each failure of an installed
    /// package's code, which may pause the package (see `pausing`). The
    /// runtime holds it weakly: it does not keep this launcher, or itself,
    /// running.
    fn report_failures(&self) {
        self.report_runtime_crashes();
        let (Ok(runtime), Some(_)) = (&self.runtime, &self.installation) else {
            return;
        };
        let launcher = self.downgrade();
        runtime.set_health(Arc::new(move |component, data, health| {
            if let Some(launcher) = launcher.upgrade() {
                launcher.note_health(component, data, health);
            }
        }));
    }

    fn downgrade(&self) -> WeakLauncher {
        WeakLauncher {
            runtime: self
                .runtime
                .as_ref()
                .map(Runtime::downgrade)
                .map_err(Clone::clone),
            commands: self.commands.clone(),
            installation: self.installation.clone(),
            links: self.links.clone(),
            hotkeys: self.hotkeys.clone(),
            sources: self.sources.clone(),
            developing: Arc::downgrade(&self.developing),
            changes: self.changes.clone(),
            background: self.background.as_ref().map(Arc::downgrade),
            state: Arc::downgrade(&self.state),
        }
    }

    pub fn view(&self) -> LauncherView {
        self.lock().view.clone()
    }

    /// The installed packages, as read from their managed copies.
    pub fn packages(&self) -> Vec<InstalledPackage> {
        self.lock().packages.clone()
    }

    /// Shows `message` as the outcome of the most recent action; for
    /// failures outside the launcher, such as a folder picker that could not
    /// open.
    pub fn show_error(&self, message: impl Into<String>) {
        self.lock().view.status = Status::Error(message.into());
    }

    /// Moves the selection by `delta` rows, clamped to the list.
    pub fn move_selection(&self, delta: isize) {
        let mut state = self.lock();
        let view = &mut state.view;
        match view.selected {
            Some(selected) => {
                let last = view.rows.len() - 1;
                view.selected = Some(selected.saturating_add_signed(delta).min(last));
            }
            // Only root search's fallbacks are listed with none selected:
            // Down chooses the first, Up the last.
            None if view.rows.is_empty() => {}
            None if delta > 0 => view.selected = Some(0),
            None => view.selected = Some(view.rows.len() - 1),
        }
    }

    /// Searches root search for `query`: the rows become the root results
    /// that match it, best match first, and the best match is selected. An
    /// empty query lists every root result. Ignored on other screens, and
    /// when `query` is already the query.
    ///
    /// Metadata is searched at once, without running any guest. For a query
    /// that is not blank, the enabled commands that compute root results
    /// (such as the calculator) are asked too, one after another: await the
    /// returned future to list each one's results, above the others, as soon
    /// as it answers. Their answers are discarded if the query has changed
    /// meanwhile, and a command that fails is listed as a result explaining
    /// the failure.
    ///
    /// The first query that is not blank since root search was shown also
    /// asks the enabled commands that supply results ahead of the query
    /// (such as the installed applications), after those; the future lists
    /// their results, matched like titles, when they answer, and they are
    /// kept for later queries. Until then the results kept from before are
    /// listed.
    ///
    /// On an open command that searches as the user types, `query` is the
    /// text of its own search field instead: see
    /// [`Launcher::search_in_command`]. Root search's providers are not
    /// asked then, and the opened command never is from root search.
    pub fn set_query(&self, query: &str) -> impl Future<Output = ()> + Send + 'static {
        let mut state = self.lock();
        let in_command = match &state.view.screen {
            Screen::CommandSearch { query: current } if current != query => {
                self.search_in_command(&mut state, query)
            }
            _ => None,
        };
        let (asked, indexing, cancelled) = match &state.view.screen {
            Screen::Root { query: current } if current != query => {
                let cancelled = self.search(&mut state, query);
                let indexing = self.ask_for_indexed_results(&mut state, query);
                (
                    self.ask_for_root_results(&state, query),
                    indexing,
                    Some(cancelled),
                )
            }
            // Searching the same query again changes nothing, not even the
            // selection.
            _ => (Vec::new(), Vec::new(), None),
        };
        let query = query.to_owned();
        let epoch = state.screen_epoch;
        let search = state.search_epoch;
        drop(state);
        let launcher = self.clone();
        async move {
            if let Some(searching) = in_command {
                searching.await;
            }
            let Some(mut cancelled) = cancelled.filter(|_| !asked.is_empty()) else {
                launcher.show_indexed_results(indexing).await;
                return;
            };
            let listing = launcher
                .show_root_results(epoch, search, &query, asked, &mut cancelled)
                .await;
            launcher.show_indexed_results(indexing).await;
            // Commands whose granted folder was still being listed are asked
            // again once it is, after every other result was shown.
            for (command, data, listed) in listing.unwrap_or_default() {
                if until_cancelled(listed, &mut cancelled).await.is_none() {
                    return;
                }
                let asked = launcher
                    .show_one_root_result(epoch, search, &query, command, data, &mut cancelled)
                    .await;
                if asked.is_none() {
                    return;
                }
            }
        }
    }

    /// The enabled commands that supply root results ahead of the query and
    /// are to be asked now, each with its settings; none for a blank query.
    fn ask_for_indexed_results(
        &self,
        state: &mut State,
        query: &str,
    ) -> Vec<(CommandRegistration, Option<PackageData>)> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        let commands = state
            .packages
            .iter()
            .filter(|package| state.runs(package))
            .flat_map(|package| {
                let data = self
                    .installation
                    .as_ref()
                    .map(|installation| installation.data.owned_by(&package.identity));
                package
                    .indexed_result_commands()
                    .into_iter()
                    .map(move |command| (command, data.clone()))
            })
            .collect();
        state.indexes.begin_asking(commands)
    }

    /// Asks each of `commands` in turn for its results ahead of the query
    /// and keeps them, listing them in root search if it is on screen,
    /// whatever the query is by then. A command disabled or replaced
    /// meanwhile contributes nothing.
    async fn show_indexed_results(
        &self,
        commands: Vec<(CommandRegistration, Option<PackageData>)>,
    ) {
        for (command, data) in commands {
            let answer = match self.runtime() {
                Ok(runtime) => {
                    runtime
                        .indexed_results_with(&command.component, data.clone())
                        .await
                }
                Err(error) => Err(error),
            };
            let mut state = self.lock();
            let state = &mut *state;
            if data.as_ref().and_then(PackageData::stopped).is_some() {
                continue;
            }
            state.indexes.answer(&command, answer);
            if let Some(query) = state.view.query().map(str::to_owned) {
                relist_root(state, &query);
            }
        }
    }

    /// Forgets the results supplied ahead of the query by commands that are
    /// no longer enabled, are paused, or were replaced.
    fn forget_indexes(state: &mut State) {
        let indexing: Vec<PathBuf> = state
            .packages
            .iter()
            .filter(|package| state.runs(package))
            .flat_map(|package| package.indexed_result_commands())
            .map(|command| command.component)
            .collect();
        state
            .indexes
            .retain(|component| indexing.iter().any(|kept| kept == component));
    }

    /// Opens the installed application `id`, named `name`, off the calling
    /// thread, and reports whether the system opened it.
    async fn open_application(&self, epoch: u64, id: String, name: String) {
        let opened = match self.runtime() {
            Ok(runtime) => {
                let applications = runtime.applications();
                off_thread(move || applications.open(&id)).await
            }
            Err(error) => Err(error.to_string()),
        };
        let Some(mut state) = self.lock_if_current(epoch) else {
            return;
        };
        state.view.status = match opened {
            Ok(()) => Status::Result(format!("Opened {name}")),
            Err(problem) => Status::Error(format!("Could not open {name}: {problem}")),
        };
    }

    /// Shows the root results matching `query` from metadata alone; results
    /// computed for an earlier query are gone, and the calls still asking
    /// for them are cancelled. Returns what resolves once this search is
    /// replaced too, or root search is left.
    fn search(&self, state: &mut State, query: &str) -> tokio::sync::oneshot::Receiver<()> {
        state.search_epoch += 1;
        let (alive, cancelled) = tokio::sync::oneshot::channel();
        // Dropping the earlier search's cancels its pending calls.
        state.search_alive = Some(alive);
        state.computed.clear();
        // A command's answer to the query sent is not an answer to this one.
        if state.sent_from.take().is_some_and(|sent| sent != query) {
            state.view.status = Status::Idle;
        }
        let (rows, entries) = root_rows(state, query);
        state.view.screen = Screen::Root {
            query: query.to_owned(),
        };
        state.view.selected = aliases::first_choice(&entries);
        state.view.rows = rows;
        state.entries = entries;
        cancelled
    }

    /// The enabled commands that compute root results, each with its
    /// extension data, to be asked for their results for `query`; none for a blank
    /// query.
    fn ask_for_root_results(
        &self,
        state: &State,
        query: &str,
    ) -> Vec<(CommandRegistration, Option<PackageData>)> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        state
            .packages
            .iter()
            .filter(|package| state.runs(package))
            .flat_map(|package| {
                let data = self
                    .installation
                    .as_ref()
                    .map(|installation| installation.data.owned_by(&package.identity));
                package
                    .root_result_commands()
                    .into_iter()
                    .map(move |command| (command, data.clone()))
            })
            .collect()
    }

    /// Asks each of `commands` in turn for its root results for `query` and
    /// lists each one's as soon as it answers, unless the query, the search
    /// or the screen has changed meanwhile.
    ///
    /// Once `cancelled` resolves (the search was replaced, or root search
    /// was left), the pending call is dropped, which cancels it in the
    /// runtime, and no further command is asked. The runtime serves calls one
    /// at a time, so a command that is slow or hangs still delays the
    /// commands asked after it until then (timeouts are #18); it no longer
    /// hides the answers of those asked before it.
    async fn show_root_results(
        &self,
        epoch: u64,
        search: u64,
        query: &str,
        commands: Vec<(CommandRegistration, Option<PackageData>)>,
        cancelled: &mut tokio::sync::oneshot::Receiver<()>,
    ) -> Option<Vec<Listing>> {
        let mut listing = Vec::new();
        for (command, data) in commands {
            let asked = self
                .show_one_root_result(
                    epoch,
                    search,
                    query,
                    command.clone(),
                    data.clone(),
                    cancelled,
                )
                .await?;
            if let Some(listed) = asked {
                listing.push((command, data, listed));
            }
        }
        Some(listing)
    }

    /// Asks `command` for its root results for `query` and lists them in
    /// place of any it gave before, unless the query, the search or the
    /// screen changed meanwhile (then `None`: ask nothing more). Answers
    /// what resolves once the granted folder its package's answer was still
    /// waiting for is listed, if it was.
    async fn show_one_root_result(
        &self,
        epoch: u64,
        search: u64,
        query: &str,
        command: CommandRegistration,
        data: Option<PackageData>,
        cancelled: &mut tokio::sync::oneshot::Receiver<()>,
    ) -> Option<Option<ListedFuture>> {
        let answer = match self.runtime() {
            Ok(runtime) => {
                let call = runtime.root_results_with(&command.component, query, data.clone());
                until_cancelled(call, cancelled).await?
            }
            Err(error) => Err(error),
        };
        let mut state = self.lock_if_current(epoch)?;
        if state.search_epoch != search || state.view.query() != Some(query) {
            return None;
        }
        let state = &mut *state;
        // A command disabled or replaced meanwhile contributes nothing.
        if data.as_ref().and_then(PackageData::stopped).is_some() {
            return Some(None);
        }
        let owner = owner(&state.packages, &command.component).map(|p| p.identity.key());
        let listed = match (&owner, &state.files) {
            (Some(owner), Some(files)) => files
                .listed(owner)
                .map(|listed| Box::pin(listed) as ListedFuture),
            _ => None,
        };
        let component = command.component.clone();
        state
            .computed
            .retain(|computed| computed.component != component);
        let files = state.files.clone();
        state.computed.extend(computed_results(
            command,
            owner.as_deref(),
            files.as_ref(),
            query,
            answer,
        ));
        relist_root(state, query);
        Some(listed)
    }

    /// Selects the row at `index`, if there is one.
    pub fn select(&self, index: usize) {
        let mut state = self.lock();
        if index < state.view.rows.len() {
            state.view.selected = Some(index);
        }
    }

    /// The text activating the selected row copies to the clipboard, if it
    /// copies. Activating it only reports the copy: the window writes the
    /// clipboard.
    pub fn selected_copy(&self) -> Option<String> {
        let state = self.lock();
        let entry = state
            .view
            .selected
            .and_then(|index| state.entries.get(index));
        match entry {
            Some(Entry::Copy(text)) => Some(text.clone()),
            _ => None,
        }
    }

    /// Whether the selected row installs a package from a folder the user
    /// chooses. Activating it does nothing in the launcher: the window asks
    /// for a folder and calls [`Launcher::preview_package`].
    pub fn selected_asks_for_folder(&self) -> bool {
        let state = self.lock();
        let entry = state
            .view
            .selected
            .and_then(|index| state.entries.get(index));
        matches!(entry, Some(Entry::InstallFromFolder))
    }

    /// Leaves an open form or custom view for its command's list, or an open
    /// command, package preview or the extension list for root search. A
    /// custom view is closed. On root search it clears the query.
    pub fn back(&self) {
        let mut state = self.lock();
        match &state.view.screen {
            Screen::Form(_) => {
                let form = state.form.take().expect("a form is open");
                state.next_screen();
                state.view = LauncherView {
                    status: Status::Idle,
                    ..form.return_to
                };
            }
            Screen::CustomView(_) => self.return_from_custom_view(&mut state, Status::Idle),
            Screen::Confirm { .. } => self.leave_confirm(&mut state),
            Screen::Hotkey { command, .. } => {
                let command = command.clone();
                self.show_extensions_at_hotkey(&mut state, &command);
            }
            Screen::PauseDetails { identity, .. } => {
                let identity = identity.clone();
                self.show_extensions_at(
                    &mut state,
                    |entry| matches!(entry, Entry::PauseDetails(shown) if *shown == identity),
                );
            }
            Screen::NetworkDetails { identity, .. } => {
                let identity = identity.clone();
                self.show_extensions_at(
                    &mut state,
                    |entry| matches!(entry, Entry::NetworkDetails(shown) if *shown == identity),
                );
            }
            Screen::BuildDetails { identity, .. } => {
                let identity = identity.clone();
                self.show_extensions_at(
                    &mut state,
                    |entry| matches!(entry, Entry::BuildDetails(shown) if *shown == identity),
                );
            }
            Screen::RuntimeDetails { .. } => {
                self.show_extensions_at(&mut state, |entry| matches!(entry, Entry::RuntimeDetails));
            }
            // Escape clears the command's search before leaving it, as it
            // clears root search's query.
            Screen::CommandSearch { query } if !query.is_empty() => {
                self.clear_search_in_command(&mut state);
            }
            Screen::Command
            | Screen::CommandSearch { .. }
            | Screen::Package { .. }
            | Screen::Extensions { .. } => self.show_root(&mut state, None),
            Screen::Root { query } => {
                if !query.is_empty() {
                    self.search(&mut state, "");
                }
            }
        }
    }

    /// Opens the selected command (root), opens the selected item's form or
    /// runs its action (command view), or installs or updates the previewed
    /// package. Await the returned future to apply the reply.
    ///
    /// A row that [asks for a folder](Launcher::selected_asks_for_folder)
    /// does nothing here. An [unavailable](Row::unavailable) item shows its
    /// reason as the status error without calling the extension.
    pub fn activate_selected(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut state = self.lock();
        let entry = state
            .view
            .selected
            .and_then(|index| state.entries.get(index).cloned());
        let mut change = None;
        let mut reload = None;
        let mut hotkey_change = None;
        let mut choice_change = None;
        let mut task_switch = None;
        let mut uninstall = None;
        let mut develop = None;
        let mut delete_retained = None;
        let mut install = None;
        let mut stop_sharing = None;
        // The status line is about this action from now on.
        state.sent_from = None;
        let entry = match entry {
            Some(Entry::Send(sending)) => match &sending.unavailable {
                Some(reason) => {
                    state.view.status = Status::Error(reason.clone());
                    None
                }
                None => {
                    state.sent_from = state.view.query().map(str::to_owned);
                    state.view.status = Status::Running;
                    Some(Entry::Send(sending))
                }
            },
            Some(Entry::Broken(problem) | Entry::Unavailable(problem)) => {
                state.view.status = Status::Error(problem);
                None
            }
            Some(Entry::Copy(text)) => {
                state.view.status = Status::Result(format!("Copied {text} to the clipboard"));
                None
            }
            Some(Entry::Form(item_id, form)) => {
                open_form(&mut state, item_id, form);
                None
            }
            Some(Entry::OpenUrl(url)) => match links::refusal(&url) {
                Some(reason) => {
                    state.view.status = Status::Error(format!("Could not open {url}: {reason}"));
                    None
                }
                None => {
                    state.view.status = Status::Running;
                    Some(Entry::OpenUrl(url))
                }
            },
            Some(Entry::StopSharingFolder(identity)) => {
                stop_sharing = Some(identity);
                None
            }
            Some(Entry::Manage) => {
                self.show_extensions(&mut state);
                None
            }
            Some(Entry::AskClearCache(identity)) => {
                self.show_clear_cache(&mut state, &identity);
                None
            }
            Some(Entry::NetworkDetails(identity)) => {
                self.show_network_details(&mut state, &identity);
                None
            }
            Some(Entry::PauseDetails(identity)) => {
                self.show_pause_details(&mut state, &identity);
                None
            }
            Some(Entry::RuntimeDetails) => {
                self.show_runtime_details(&mut state);
                None
            }
            Some(Entry::RestartRuntime) => {
                self.restart_runtime(&mut state);
                None
            }
            Some(Entry::Develop(identity)) => {
                develop = self
                    .begin_developing(&mut state, &identity)
                    .map(|start| (identity, start));
                None
            }
            Some(Entry::StopDeveloping(identity)) => {
                self.end_developing(&mut state, &identity);
                self.refresh(&mut state);
                None
            }
            Some(Entry::BuildDetails(identity)) => {
                self.show_build_details(&mut state, &identity);
                None
            }
            Some(Entry::BuildAgain(identity)) => {
                self.build_again(&mut state, &identity);
                None
            }
            Some(Entry::AskUninstall(identity)) => {
                let closure = dependencies::required_dependents(&state.packages, &identity);
                if closure.is_empty() {
                    self.show_uninstall(&mut state, &identity);
                } else {
                    self.show_uninstall_dependents(&mut state, &identity, closure);
                }
                None
            }
            Some(Entry::Uninstall(identity, saved)) => {
                uninstall = self.begin_uninstall(&mut state, vec![identity], saved);
                None
            }
            Some(Entry::UninstallAll(identity, shown, saved)) => {
                uninstall = self.begin_uninstall_all(&mut state, identity, &shown, saved);
                None
            }
            Some(Entry::AskDeleteRetained(identity)) => {
                self.show_delete_retained(&mut state, &identity);
                None
            }
            Some(Entry::DeleteRetained(identity)) => {
                delete_retained = self.begin_delete_retained(&mut state, identity);
                None
            }
            Some(Entry::Cancel) => {
                self.leave_confirm(&mut state);
                None
            }
            Some(Entry::AskHotkey(command)) => {
                self.show_hotkey(&mut state, &command);
                None
            }
            Some(Entry::RemoveHotkey(command)) => {
                hotkey_change = self.remove_hotkey(&mut state, &command);
                None
            }
            Some(Entry::AskAlias(command)) => {
                self.show_alias_form(&mut state, &command);
                None
            }
            Some(Entry::ToggleFallback(command)) => {
                choice_change = Some(self.toggle_fallback(&mut state, &command));
                None
            }
            Some(Entry::ForgetChoices(command)) => {
                choice_change = Some(self.forget_choices(&mut state, &command));
                None
            }
            Some(Entry::ToggleTask(command)) => {
                task_switch = self.toggle_task(&mut state, &command);
                None
            }
            Some(Entry::Toggle(identity)) => {
                // The package's state when the user pressed, not when the
                // future runs.
                let enable = state.package(&identity).is_some_and(|p| !p.enabled);
                let closure = match enable {
                    true => Vec::new(),
                    false => dependencies::required_dependents(&state.packages, &identity),
                };
                if closure.iter().any(|dependent| dependent.enabled) {
                    self.show_disable_dependents(&mut state, &identity, closure);
                } else {
                    change = self.begin_change(&mut state, vec![identity], enable);
                }
                None
            }
            Some(Entry::DisableAll(identity, shown)) => {
                change = self.begin_disable_all(&mut state, identity, &shown);
                None
            }
            Some(Entry::Reload(identity)) => {
                reload = self.begin_reload(&mut state, identity, reload::Attempt::Reload);
                None
            }
            Some(Entry::Retry(identity)) => {
                reload = self.begin_reload(&mut state, identity, reload::Attempt::Retry);
                None
            }
            Some(Entry::Install(request, mode, assumptions)) => {
                install = self.begin_install(&mut state, request, mode, assumptions);
                None
            }
            Some(Entry::AskNpm) => {
                self.show_npm_form(&mut state);
                None
            }
            Some(Entry::InstallFromFolder | Entry::ChooseFolder(_)) | None => None,
            Some(entry) => {
                state.view.status = Status::Running;
                Some(entry)
            }
        };
        let epoch = state.screen_epoch;
        let open = state.open.clone();
        // A call into the package belongs to its generation as of now, not
        // as of when the returned future first runs.
        let called = match &entry {
            Some(
                Entry::Open(Opening { component, .. })
                | Entry::Send(aliases::Sending { component, .. }),
            ) => Some(component),
            Some(Entry::Run(_) | Entry::CustomView(..)) => open.as_ref(),
            _ => None,
        };
        let data = called.and_then(|component| self.data_in(&state, component));
        drop(state);
        let launcher = self.clone();
        async move {
            if let Some((identity, start)) = develop {
                launcher.finish_developing(identity, start).await;
            }
            if let Some(change) = change {
                launcher.finish_change(epoch, change).await;
            }
            if let Some(reload) = reload {
                launcher.finish_reload(epoch, reload).await;
            }
            if let Some(hotkey_change) = hotkey_change {
                launcher.finish_hotkey_change(hotkey_change).await;
            }
            if let Some(choice_change) = choice_change {
                launcher.finish_choice_change(choice_change).await;
            }
            if let Some(task_switch) = task_switch {
                launcher.finish_task_switch(task_switch).await;
            }
            if let Some(uninstall) = uninstall {
                launcher.finish_uninstall(epoch, uninstall).await;
            }
            if let Some(retained) = delete_retained {
                launcher.finish_delete_retained(epoch, retained).await;
            }
            if let Some(install) = install {
                launcher.finish_install(epoch, install).await;
            }
            if let Some(identity) = stop_sharing {
                launcher.stop_sharing_folder(identity).await;
            }
            match entry {
                Some(Entry::Open(opening)) => launcher.open_command(epoch, opening, data).await,
                Some(Entry::Send(sending)) => launcher.run_query(epoch, sending, data).await,
                Some(Entry::OpenApplication { id, name }) => {
                    launcher.open_application(epoch, id, name).await
                }
                Some(Entry::Run(item_id)) => {
                    if let Some(component) = open {
                        launcher.run_action(epoch, component, item_id, data).await
                    }
                }
                Some(Entry::OpenUrl(url)) => launcher.open_url(epoch, url).await,
                Some(Entry::OpenFile { owner, id, name }) => {
                    launcher.open_file(epoch, owner, id, name).await
                }
                Some(Entry::ClearCache(identity)) => launcher.clear_cache(epoch, identity).await,
                Some(Entry::CustomView(item_id, info)) => {
                    if let Some(component) = open {
                        launcher
                            .open_custom_view(epoch, component, item_id, info, data)
                            .await
                    }
                }
                Some(
                    Entry::Copy(_)
                    | Entry::Broken(_)
                    | Entry::Unavailable(_)
                    | Entry::InstallFromFolder
                    | Entry::AskNpm
                    | Entry::ChooseFolder(_)
                    | Entry::StopSharingFolder(_)
                    | Entry::Install(..)
                    | Entry::Manage
                    | Entry::Toggle(_)
                    | Entry::DisableAll(..)
                    | Entry::Reload(_)
                    | Entry::Retry(_)
                    | Entry::PauseDetails(_)
                    | Entry::NetworkDetails(_)
                    | Entry::RuntimeDetails
                    | Entry::RestartRuntime
                    | Entry::Develop(_)
                    | Entry::StopDeveloping(_)
                    | Entry::BuildDetails(_)
                    | Entry::BuildAgain(_)
                    | Entry::AskClearCache(_)
                    | Entry::AskHotkey(_)
                    | Entry::RemoveHotkey(_)
                    | Entry::AskAlias(_)
                    | Entry::ToggleFallback(_)
                    | Entry::ForgetChoices(_)
                    | Entry::ToggleTask(_)
                    | Entry::AskUninstall(_)
                    | Entry::Uninstall(..)
                    | Entry::UninstallAll(..)
                    | Entry::AskDeleteRetained(_)
                    | Entry::DeleteRetained(_)
                    | Entry::Cancel
                    | Entry::Form(..),
                )
                | None => {}
            }
        }
    }

    /// Reads the package in `folder` and shows its identity, version,
    /// commands and compatibility, offering Install, or Update when a
    /// package with the same identity is installed. No guest code runs. An
    /// invalid or incompatible package is explained instead.
    pub fn preview_package(&self, folder: &Path) -> impl Future<Output = ()> + Send + 'static {
        self.preview(Ok(install::Request::Folder(folder.to_path_buf())))
    }

    /// Downloads the npm package `spec` names (`name`, `@scope/name`, with
    /// an optional exact version: `name@1.2.3`) and shows it as
    /// [`Launcher::preview_package`] shows a folder, with the npm version it
    /// would install. Without a version it is the latest; with one,
    /// installing pins the package to it. Nothing in the package runs.
    pub fn preview_npm(&self, spec: &str) -> impl Future<Output = ()> + Send + 'static {
        let request = crate::npm::NpmSpec::parse(spec)
            .map(install::Request::Npm)
            .map_err(|why| (spec.trim().to_owned(), why));
        self.preview(request)
    }

    /// Previews the package `request` names, or explains why the text asked
    /// for (and the reason) names none.
    fn preview(
        &self,
        request: Result<install::Request, (String, String)>,
    ) -> impl Future<Output = ()> + Send + 'static {
        let epoch = self.start_running();
        let launcher = self.clone();
        async move {
            let request = match request {
                Ok(request) => request,
                Err((asked, why)) => {
                    let mut state = launcher.lock();
                    if state.screen_epoch == epoch {
                        launcher.leave_command(&mut state);
                        state.entries = Vec::new();
                        state.view = LauncherView {
                            status: Status::Error(why),
                            ..LauncherView::new(
                                Screen::Package {
                                    details: vec![format!("npm package: {asked}")],
                                },
                                format!("Cannot install {asked}"),
                            )
                        };
                    }
                    return;
                }
            };
            let request = launcher.keeping_pin(request);
            let checked = match launcher.read_and_check(request.clone()).await {
                Ok(package) => Ok(launcher.plan_dependencies(package).await),
                Err(error) => Err(error),
            };
            let mut state = launcher.lock();
            if state.screen_epoch != epoch {
                return;
            }
            launcher.show_preview(&mut state, &request, checked);
        }
    }

    /// `request`, or for an npm package without a version that is installed
    /// pinned to one, that version: updating it keeps its pin, which only
    /// naming another version changes.
    fn keeping_pin(&self, request: install::Request) -> install::Request {
        match request {
            install::Request::Npm(spec) if spec.version.is_none() => {
                let state = self.lock();
                let pinned = state
                    .package(&PackageIdentity::npm(&spec.name))
                    .and_then(|package| package.npm.as_ref())
                    .filter(|npm| npm.pinned)
                    .map(|npm| npm.version.clone());
                install::Request::Npm(crate::npm::NpmSpec {
                    version: pinned,
                    ..spec
                })
            }
            other => other,
        }
    }

    /// Shows the package screen for `request`, from its package and plan or
    /// why it cannot be read.
    fn show_preview(
        &self,
        state: &mut State,
        request: &install::Request,
        checked: Result<(SourcePackage, dependencies::Plan), PackageError>,
    ) {
        let installed = checked
            .as_ref()
            .ok()
            .and_then(|(package, _)| state.package(&package.identity).cloned());
        let (view, entries) = preview_view(request, checked, installed);
        self.leave_command(state);
        state.view = view;
        state.entries = entries;
    }

    /// Shows Pane's own form asking which npm package to install.
    fn show_npm_form(&self, state: &mut State) {
        let form = FormView {
            fields: vec![FormField {
                id: NPM_PACKAGE_FIELD.into(),
                label: "npm package: its name, and a version to install that one".into(),
                kind: FieldKind::Text {
                    placeholder: Some("such as @scope/name or name@1.2.3".into()),
                },
                value: String::new(),
                error: None,
            }],
            submit_label: "Show package".into(),
        };
        let view = LauncherView::new(Screen::Form(form), "Install extension from npm");
        let return_to = std::mem::replace(&mut state.view, view);
        state.form = Some(OpenForm {
            purpose: FormPurpose::Npm,
            return_to,
            submitting: false,
        });
        state.screen_epoch += 1;
    }

    /// Installs the package in `folder` as an explicit install request, with
    /// the required dependencies it is missing, as the preview would show
    /// them. A package whose identity is already installed is rejected:
    /// replacing it is an update.
    pub fn install_package(&self, folder: &Path) -> impl Future<Output = ()> + Send + 'static {
        self.install_unplanned(Ok(install::Request::Folder(folder.to_path_buf())))
    }

    /// Installs the npm package `spec` names as an explicit install request,
    /// as [`Launcher::install_package`] installs a folder: a package whose
    /// npm name is already installed is rejected, whatever its version.
    pub fn install_npm(&self, spec: &str) -> impl Future<Output = ()> + Send + 'static {
        self.install_unplanned(crate::npm::NpmSpec::parse(spec).map(install::Request::Npm))
    }

    fn install_unplanned(
        &self,
        request: Result<install::Request, String>,
    ) -> impl Future<Output = ()> + Send + 'static {
        let epoch = self.start_running();
        let launcher = self.clone();
        async move {
            match request {
                Ok(request) => {
                    let install = install::Begun::unplanned(request);
                    launcher.finish_install(epoch, install).await
                }
                Err(why) => {
                    if let Some(mut state) = launcher.lock_if_current(epoch) {
                        state.view.status = Status::Error(why);
                    }
                }
            }
        }
    }

    /// Enables or disables the installed package with `identity` and
    /// records the choice, so it holds after a restart. The choice applies at
    /// once: a disabled package's commands leave root search, an open one
    /// closes, its running instances are dropped and it can no longer save
    /// settings, even before the choice is on disk. Its settings are kept for
    /// when it is enabled again. Other installations, even with the same
    /// title, are unaffected. Await the returned future to record the choice;
    /// if it cannot be recorded, the package returns to its previous state.
    ///
    /// While an earlier change to the same package is being recorded, this
    /// does nothing.
    pub fn set_enabled(
        &self,
        identity: &PackageIdentity,
        enabled: bool,
    ) -> impl Future<Output = ()> + Send + 'static {
        let mut state = self.lock();
        let change = self.begin_change(&mut state, vec![identity.clone()], enabled);
        let epoch = state.screen_epoch;
        drop(state);
        let launcher = self.clone();
        async move {
            if let Some(change) = change {
                launcher.finish_change(epoch, change).await;
            }
        }
    }

    /// Applies the user's choice to enable or disable the packages with
    /// `identities` (the one asked about first), to be recorded together by
    /// [`Launcher::finish_change`]. Changes none of them, explains why and
    /// returns `None` if one is not installed or something else is happening
    /// to it; returns `None` without a word while another change to one is
    /// being recorded.
    fn begin_change(
        &self,
        state: &mut State,
        identities: Vec<PackageIdentity>,
        enabled: bool,
    ) -> Option<Change> {
        if self.installation.is_none() {
            let error = PackageError::Storage("this launcher does not install packages".into());
            state.view.status = Status::Error(error.to_string());
            return None;
        }
        if let Some(missing) = identities.iter().find(|i| state.package(i).is_none()) {
            let error = PackageError::NotInstalled(missing.clone());
            state.view.status = Status::Error(error.to_string());
            return None;
        }
        let claims: Vec<(PackageIdentity, Changing)> = identities
            .iter()
            .map(|identity| (identity.clone(), Changing::Recording))
            .collect();
        match state.claim_all(&claims) {
            Ok(()) => {}
            // A second enabling or disabling while one is recorded is
            // ignored without a word, as pressing Enter twice would do.
            Err((_, Changing::Recording)) => return None,
            Err((identity, busy)) => {
                let message = format!("{} {}", state.title_of(&identity), busy.doing());
                state.view.status = Status::Error(message);
                return None;
            }
        }
        for identity in &identities {
            self.apply_enabled(state, identity, enabled);
        }
        state.view.status = Status::Running;
        Some(Change {
            identities,
            enabled,
        })
    }

    /// Records a change begun by [`Launcher::begin_change`] in one write,
    /// undoing all of it if it cannot be recorded.
    async fn finish_change(&self, epoch: u64, change: Change) {
        let Change {
            identities,
            enabled,
        } = change;
        let store = self
            .installation
            .as_ref()
            .expect("begin_change checked there is an installation")
            .store
            .clone();
        let recorded = {
            let identities = identities.clone();
            off_thread(move || {
                let mut store = store.lock().unwrap_or_else(|p| p.into_inner());
                store.set_enabled_all(&identities, enabled)
            })
            .await
        };
        let mut state = self.lock();
        for identity in &identities {
            state.release(identity);
        }
        let status = match recorded {
            Ok(()) => {
                let titles: Vec<String> = identities.iter().map(|i| state.title_of(i)).collect();
                match (enabled, titles.as_slice()) {
                    (true, _) => Status::Result(format!("Enabled {}", platform::join(&titles))),
                    (false, [title]) => Status::Result(format!("Disabled {title}")),
                    (false, [title, dependents @ ..]) => {
                        Status::Result(dependents::disabled(title, dependents))
                    }
                    (false, []) => Status::Idle,
                }
            }
            Err(error) => {
                for identity in &identities {
                    self.apply_enabled(&mut state, identity, !enabled);
                }
                Status::Error(error.to_string())
            }
        };
        if state.screen_epoch == epoch {
            state.view.status = status;
        }
    }

    /// Enables or disables the package with `identity` in this launcher,
    /// without recording it: whether it offers commands and may save
    /// settings, its instances, and the screen showing them.
    fn apply_enabled(&self, state: &mut State, identity: &PackageIdentity, enabled: bool) {
        let Some(package) = state.packages.iter_mut().find(|p| p.identity == *identity) else {
            return;
        };
        package.enabled = enabled;
        let components: Vec<PathBuf> = package
            .commands()
            .into_iter()
            .map(|command| command.component)
            .collect();
        if let Some(installation) = &self.installation {
            installation.data.set_enabled(identity, enabled);
        }
        // Disabling or enabling ends a pause: the package starts afresh
        // (recording the choice forgets it too).
        self.unpause(state, identity);
        // Its hotkeys are released while it is disabled.
        self.sync_hotkeys(state);
        if !enabled {
            // Its development ends, with a build that is running.
            self.developing.end(Some(identity));
            // Its results kept for root search go, and so does an answer
            // from it being awaited.
            Launcher::forget_indexes(state);
            // Its instances stop; enabling it again starts fresh ones.
            if let Ok(runtime) = self.runtime() {
                runtime.forget(components.iter().cloned());
            }
            if state
                .open
                .as_ref()
                .is_some_and(|open| components.contains(open))
            {
                self.show_root(state, None);
                return;
            }
        }
        self.refresh(state);
    }

    fn start_running(&self) -> u64 {
        let mut state = self.lock();
        state.view.status = Status::Running;
        state.screen_epoch
    }

    /// Records `installed` as the managed copy of its package: added, or
    /// replacing the copy before it, whose instances stop. Returns whether a
    /// command of the replaced copy is open; the caller leaves it, since its
    /// state is not carried over to the new code.
    fn put_installed(&self, state: &mut State, installed: InstalledPackage) -> bool {
        // New code has not failed.
        self.unpause(state, &installed.identity);
        let Some(package) = state
            .packages
            .iter_mut()
            .find(|package| package.identity == installed.identity)
        else {
            // An identity installed again after it was uninstalled may save
            // data again, and data retained for it is its own again.
            state
                .retained
                .retain(|retained| retained.identity != installed.identity);
            if let Some(installation) = &self.installation {
                installation
                    .data
                    .set_enabled(&installed.identity, installed.enabled);
            }
            state.packages.push(installed);
            self.sync_hotkeys(state);
            return false;
        };
        // The replaced copy's code no longer runs: its generation ended
        // before its folder was removed ([`Launcher::retire`]).
        let replaced: Vec<PathBuf> = package
            .commands()
            .into_iter()
            .map(|c| c.component)
            .collect();
        if let Ok(runtime) = self.runtime() {
            runtime.forget(replaced.iter().cloned());
        }
        *package = installed;
        // A command the new copy no longer has releases its hotkey.
        self.sync_hotkeys(state);
        // The replaced copy's results are asked for afresh.
        state
            .indexes
            .retain(|component| !replaced.iter().any(|old| old == component));
        state
            .open
            .as_ref()
            .is_some_and(|open| replaced.contains(open))
    }

    /// Reads the package `request` names off the calling thread (for npm,
    /// downloading it), then has the runtime check each component without
    /// running it.
    async fn read_and_check(
        &self,
        request: install::Request,
    ) -> Result<SourcePackage, PackageError> {
        let sources = self.sources.clone();
        let mut package = off_thread(move || sources.read(&request)).await?;
        package.network = self.check_components(&package).await?;
        Ok(package)
    }

    /// Like [`Launcher::read_and_check`], for a package staged in `folder`
    /// (a development build) that belongs to the source `identity`.
    async fn read_and_check_staged(
        &self,
        folder: PathBuf,
        identity: PackageIdentity,
    ) -> Result<SourcePackage, PackageError> {
        let mut package = off_thread(move || SourcePackage::read_staged(&folder, identity)).await?;
        package.network = self.check_components(&package).await?;
        Ok(package)
    }

    /// Has the runtime check each component of `package` without running
    /// it; whether any imports `wasi:http` (it can make web requests).
    async fn check_components(&self, package: &SourcePackage) -> Result<bool, PackageError> {
        let mut network = false;
        let mut checked_components = Vec::new();
        for (name, component) in package.manifest.components() {
            // A component serving several commands or operations is checked
            // once, for everything it serves.
            if checked_components.contains(&component) {
                continue;
            }
            checked_components.push(component);
            let exports = package.manifest.exports_of(component);
            let source = package.folder.join(component);
            let checked = match self.runtime() {
                Ok(runtime) => runtime.check_with(&source, exports).await,
                Err(error) => Err(error),
            };
            let checked = checked.map_err(|error| PackageError::Component {
                command: name.clone(),
                error,
            })?;
            network |= checked.network;
        }
        Ok(network)
    }

    /// Shows root search with an empty query: this build's commands, then
    /// the installed packages' commands, then the install row. Selects the
    /// command with component `select` if given, else the first row.
    fn show_root(&self, state: &mut State, select: Option<PathBuf>) {
        state.root = self.root_results(state);
        state.sent_from = None;
        state.computed.clear();
        state.indexes.stale();
        let (rows, entries) = root_rows(state, "");
        let selected = select
            .and_then(|component| {
                entries.iter().position(
                    |entry| matches!(entry, Entry::Open(opening) if opening.component == component),
                )
            })
            .or_else(|| aliases::first_choice(&entries));
        self.leave_command(state);
        state.entries = entries;
        state.view = LauncherView {
            rows,
            selected,
            status: match &state.store_problem {
                Some(problem) => Status::Error(problem.clone()),
                None => Status::Idle,
            },
            ..LauncherView::new(
                Screen::Root {
                    query: String::new(),
                },
                "Pane",
            )
        };
        self.show_kept_development_status(state);
    }

    /// Updates root search or the extension list on screen after a package
    /// changed; other screens show no package state.
    fn refresh(&self, state: &mut State) {
        match &state.view.screen {
            Screen::Root { .. } => self.refresh_root(state),
            Screen::Extensions { .. } => self.refresh_extensions(state),
            Screen::Command
            | Screen::CommandSearch { .. }
            | Screen::Package { .. }
            | Screen::Form(_)
            | Screen::CustomView(_)
            | Screen::Confirm { .. }
            | Screen::Hotkey { .. } => {}
            Screen::RuntimeDetails { .. } => self.keep_runtime_details(state),
            // Once the build succeeded or development ended, the extension
            // list; else the latest failure. The screen epoch is kept.
            Screen::BuildDetails { identity, .. } => {
                let identity = identity.clone();
                let epoch = state.screen_epoch;
                self.show_build_details(state, &identity);
                state.screen_epoch = epoch;
            }
            // Once the package is no longer paused (retried, reloaded,
            // disabled), the extension list, keeping the screen epoch as
            // refreshing does.
            Screen::PauseDetails { identity, .. } if !state.paused.is_paused(identity) => {
                let identity = identity.clone();
                let epoch = state.screen_epoch;
                self.show_extensions_at(
                    state,
                    |entry| matches!(entry, Entry::Reload(shown) if *shown == identity),
                );
                state.screen_epoch = epoch;
            }
            Screen::PauseDetails { .. } => {}
            // What it reached since, or the extension list once it is gone,
            // keeping the screen epoch.
            Screen::NetworkDetails { identity, .. } => {
                let identity = identity.clone();
                let epoch = state.screen_epoch;
                self.show_network_details(state, &identity);
                state.screen_epoch = epoch;
            }
        }
    }

    /// Updates the rows of the root search on screen after the installed
    /// packages changed in the background. Unlike navigating, it keeps the
    /// screen epoch, so an action the user started from root still
    /// applies, and it keeps the query and the selection on the same row.
    fn refresh_root(&self, state: &mut State) {
        let selected_id = state
            .view
            .selected
            .and_then(|index| state.view.rows.get(index))
            .map(|row| row.id.clone());
        state.root = self.root_results(state);
        // A command that was disabled, paused or replaced contributes
        // nothing more; one enabled again answers from the next change of the
        // query.
        let computing: Vec<PathBuf> = state
            .packages
            .iter()
            .filter(|package| state.runs(package))
            .flat_map(|package| package.root_result_commands())
            .map(|command| command.component)
            .collect();
        state
            .computed
            .retain(|computed| computing.contains(&computed.component));
        Launcher::forget_indexes(state);
        let query = state.view.query().unwrap_or_default();
        let (rows, entries) = root_rows(state, query);
        let selected = selected_id
            .and_then(|id| rows.iter().position(|row| row.id == id))
            .or_else(|| aliases::first_choice(&entries));
        state.entries = entries;
        state.view.rows = rows;
        state.view.selected = selected;
    }

    /// Every root result, in root search order: this build's commands, the
    /// enabled packages' commands, the packages that cannot load, then
    /// Pane's own rows.
    fn root_results(&self, state: &State) -> Vec<RootResult> {
        let mut results = Vec::new();
        let mut add = |row: Row, entry: Entry, package: Option<&str>, target| {
            let alias = state.aliases.chosen.active_alias(&row.id);
            let keys = Keys::new(&row.title, row.subtitle.as_deref(), package).with_alias(alias);
            results.push(RootResult {
                row,
                entry,
                keys,
                target,
            });
        };
        let command = |(command, unavailable): (CommandRegistration, Option<Unavailable>)| {
            let entry = match &unavailable {
                Some(reason) => Entry::Unavailable(reason.reason().to_owned()),
                None => Entry::Open(Opening::of(&command)),
            };
            let row = Row {
                id: command.id,
                title: command.title,
                subtitle: command.subtitle,
                unavailable,
            };
            (row, entry)
        };
        // A disabled package contributes nothing to root search.
        let enabled = || state.packages.iter().filter(|package| package.enabled);
        for built in self.commands.iter().cloned() {
            let (row, entry) = command((built, None));
            add(row, entry, None, None);
        }
        for package in enabled() {
            // Its commands are found by its title too, even those that show
            // a subtitle of their own.
            let title = package.title();
            // A paused package's commands stay listed, saying why they do
            // not run.
            let paused = state
                .paused
                .is_paused(&package.identity)
                .then(|| Unavailable::Paused(paused_reason(&title)));
            for (registration, unavailable) in package.available_commands() {
                let unavailable = paused
                    .clone()
                    .or(unavailable.map(Unavailable::OnThisSystem));
                let target = aliases::Target {
                    registration: registration.clone(),
                    identity: package.identity.clone(),
                    unavailable: unavailable.clone(),
                };
                let (row, entry) = command((registration, unavailable));
                add(row, entry, Some(&title), Some(target));
            }
        }
        for package in enabled() {
            if let Err(error) = &package.manifest {
                let row = Row {
                    id: package.identity.key(),
                    title: package.title(),
                    subtitle: Some("Cannot load this installed extension".into()),
                    unavailable: None,
                };
                let problem = format!(
                    "{} cannot load from {}: {error}",
                    package.title(),
                    package.location.display()
                );
                add(row, Entry::Broken(problem), None, None);
            }
        }
        if self.installation.is_some() {
            let row = Row {
                id: INSTALL_FROM_FOLDER.into(),
                title: "Install extension from folder…".into(),
                subtitle: Some("Choose a local extension package to install".into()),
                unavailable: None,
            };
            add(row, Entry::InstallFromFolder, None, None);
            let row = Row {
                id: INSTALL_FROM_NPM.into(),
                title: "Install extension from npm…".into(),
                subtitle: Some("Download an extension package published to npm".into()),
                unavailable: None,
            };
            add(row, Entry::AskNpm, None, None);
        }
        // Retained data is managed there too, while nothing is installed.
        if self.installation.is_some() && !(state.packages.is_empty() && state.retained.is_empty())
        {
            let row = Row {
                id: MANAGE_EXTENSIONS.into(),
                title: "Manage extensions…".into(),
                subtitle: Some("Enable or disable installed extensions".into()),
                unavailable: None,
            };
            add(row, Entry::Manage, None, None);
        }
        results
    }

    /// Sets the value of the open form's field `field_id`: a text field's
    /// text, or the id of an option of a choice field. Unknown fields and
    /// options are ignored. Editing a field clears its error.
    pub fn set_field_value(&self, field_id: &str, value: &str) {
        let mut state = self.lock();
        let Screen::Form(form) = &mut state.view.screen else {
            return;
        };
        let Some(field) = form.fields.iter_mut().find(|field| field.id == field_id) else {
            return;
        };
        if let FieldKind::Choice(choices) = &field.kind
            && !choices.iter().any(|choice| choice.id == value)
        {
            return;
        }
        field.value = value.to_owned();
        field.error = None;
    }

    /// Submits the open form to its extension. Await the returned future to
    /// apply the reply: the answer as the result, or the extension's
    /// rejection next to its field. While a submission is waiting for its
    /// reply, submitting again does nothing.
    ///
    /// Pane's own alias form is applied at once instead (see `aliases`);
    /// the future records it.
    pub fn submit_form(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut state = self.lock();
        let state = &mut *state;
        let alias_change = match (&state.view.screen, &state.form) {
            (
                Screen::Form(_),
                Some(OpenForm {
                    purpose: FormPurpose::Alias(command),
                    ..
                }),
            ) => {
                let command = command.clone();
                self.submit_alias(state, &command)
            }
            _ => None,
        };
        // Pane's own npm form previews the package it names; the form stays
        // until the preview replaces it, and Back meanwhile discards it.
        let npm = match (&state.view.screen, &mut state.form) {
            (
                Screen::Form(form),
                Some(
                    open @ OpenForm {
                        purpose: FormPurpose::Npm,
                        submitting: false,
                        ..
                    },
                ),
            ) => {
                open.submitting = true;
                state.view.status = Status::Running;
                form.fields.first().map(|field| field.value.clone())
            }
            _ => None,
        };
        let submission = match (&state.view.screen, &mut state.form, &state.open) {
            (
                Screen::Form(form),
                Some(
                    open @ OpenForm {
                        purpose: FormPurpose::Item(_),
                        ..
                    },
                ),
                Some(component),
            ) if !open.submitting => {
                let FormPurpose::Item(item_id) = &open.purpose else {
                    unreachable!("matched above");
                };
                let item_id = item_id.clone();
                open.submitting = true;
                let values: Vec<FieldValue> = form
                    .fields
                    .iter()
                    .map(|field| FieldValue {
                        id: field.id.clone(),
                        value: field.value.clone(),
                    })
                    .collect();
                Some((component.clone(), item_id, values))
            }
            _ => None,
        };
        if submission.is_some() {
            state.view.status = Status::Running;
        }
        let epoch = state.screen_epoch;
        let data = submission
            .as_ref()
            .and_then(|(component, ..)| self.data_in(state, component));
        let launcher = self.clone();
        async move {
            if let Some(change) = alias_change {
                launcher.finish_choice_change(change).await;
            }
            if let Some(spec) = npm {
                launcher.preview_npm(&spec).await;
            }
            if let Some((component, item_id, values)) = submission {
                launcher
                    .submit(epoch, component, item_id, values, data)
                    .await
            }
        }
    }

    /// Sends `values` and applies the reply as though it had arrived before
    /// any edit made meanwhile: a rejected field that the user has changed
    /// since is not marked, since editing a field clears its error.
    async fn submit(
        &self,
        epoch: u64,
        component: PathBuf,
        item_id: String,
        values: Vec<FieldValue>,
        data: Option<PackageData>,
    ) {
        let result = match self.runtime() {
            Ok(runtime) => {
                runtime
                    .submit_form_with(&component, &item_id, values.clone(), data.clone())
                    .await
            }
            Err(error) => Err(error),
        };
        let Some(mut state) = self.lock_if_current(epoch) else {
            return;
        };
        let state = &mut *state;
        state.form.as_mut().expect("a form is open").submitting = false;
        if let Some(problem) = stopped(state, &component, &data) {
            // Stopped while it was submitting: its answer is not shown.
            state.view.status = Status::Error(problem);
            return;
        }
        let view = &mut state.view;
        let Screen::Form(form) = &mut view.screen else {
            unreachable!("a form is open");
        };
        let fields = &mut form.fields;
        for field in fields.iter_mut() {
            field.error = None;
        }
        view.status = match result {
            Ok(answer) => Status::Result(answer),
            Err(CallError::Form(error)) => {
                let field = error
                    .field
                    .as_deref()
                    .and_then(|id| fields.iter_mut().find(|field| field.id == id));
                match field {
                    Some(field) => {
                        let status = format!("{}: {}", field.label, error.message);
                        let unchanged = values
                            .iter()
                            .any(|sent| sent.id == field.id && sent.value == field.value);
                        if unchanged {
                            field.error = Some(error.message);
                        }
                        Status::Error(status)
                    }
                    // A rejection of the form as a whole is the extension's
                    // message to the user, shown as it is.
                    None => Status::Error(error.message),
                }
            }
            Err(error) => Status::Error(error.to_string()),
        };
    }

    /// Shows the installed packages, each enabled or disabled.
    fn show_extensions(&self, state: &mut State) {
        let (rows, entries) = self.extension_rows(state);
        self.leave_command(state);
        state.entries = entries;
        let mut details = vec![
            "A disabled extension adds no commands and runs nothing; it keeps its settings.".into(),
            "Reloading replaces an extension's code with its source folder's current build; it \
             keeps its settings."
                .into(),
            "Clearing an extension's cache keeps its settings, content and credentials.".into(),
            "Uninstalling an extension asks whether to keep its settings and content.".into(),
            format!(
                "An extension that cannot start, or crashes {}, is paused until you retry it; \
                 it keeps its settings.",
                pausing::within()
            ),
        ];
        if !state.retained.is_empty() {
            details.push(
                "Data kept for an uninstalled extension is listed until you delete it or install \
                 it again from the same source."
                    .into(),
            );
        }
        state.view =
            LauncherView::new(Screen::Extensions { details }, "Extensions").with_rows(rows);
        self.show_kept_development_status(state);
    }

    /// The extension list's rows: each package's state, reload and cache
    /// rows, then the hotkey of each command of the enabled packages, then
    /// one row per identity with retained data.
    fn extension_rows(&self, state: &State) -> (Vec<Row>, Vec<Entry>) {
        let developed = |identity: &PackageIdentity| self.is_developed(identity);
        let (mut rows, mut entries): (Vec<Row>, Vec<Entry>) =
            self.runtime_rows().into_iter().unzip();
        let (package_rows, package_entries) =
            extension_rows(&state.packages, &state.paused, developed);
        rows.extend(package_rows);
        entries.extend(package_entries);
        for (row, entry) in self
            .network_rows(state)
            .into_iter()
            .chain(self.schedule_rows(state))
        {
            rows.push(row);
            entries.push(entry);
        }
        let development = self.development_rows(&state.packages);
        for (row, entry) in self
            .hotkey_rows(state)
            .into_iter()
            .chain(self.choice_rows(state))
            .chain(development)
        {
            rows.push(row);
            entries.push(entry);
        }
        if let Some(installation) = &self.installation {
            let (retained_rows, retained_entries) =
                retained::rows(&state.retained, &installation.data);
            rows.extend(retained_rows);
            entries.extend(retained_entries);
        }
        (rows, entries)
    }

    /// Shows why Pane paused the installed package with `identity`: how it
    /// failed, its version and the full diagnostics, with a row that retries
    /// it. A package that is not paused (it was retried meanwhile) shows the
    /// extension list instead.
    fn show_pause_details(&self, state: &mut State, identity: &PackageIdentity) {
        let (Some(package), Some(pause)) = (state.package(identity), state.paused.of(identity))
        else {
            self.show_extensions(state);
            return;
        };
        let title = package.title();
        let mut details = vec![
            format!("{}.", pausing::failure(&title, pause.after)),
            format!("From {identity}"),
        ];
        if let Some(version) = &pause.version {
            details.push(format!("Version: {version}"));
        }
        details.push(
            "Pane runs none of its code until you retry it, reload or update it, or disable and \
             enable it. Its settings and saved data are kept."
                .into(),
        );
        details.extend(
            pause
                .why
                .lines()
                .map(str::trim_end)
                .filter(|line| !line.is_empty())
                .map(str::to_owned),
        );
        let retry = Row {
            id: format!("retry:{}", identity.key()),
            title: pausing::retry_title(&title, pause.after),
            subtitle: Some("Start it again".into()),
            unavailable: None,
        };
        state.next_screen();
        state.entries = vec![Entry::Retry(identity.clone())];
        let screen = Screen::PauseDetails {
            identity: identity.clone(),
            details,
        };
        state.view =
            LauncherView::new(screen, pausing::details_title(&title)).with_rows(vec![retry]);
    }

    /// Asks whether to clear the cache of the installed package with
    /// `identity`, saying what is deleted and what is kept.
    fn show_clear_cache(&self, state: &mut State, identity: &PackageIdentity) {
        let title = state.title_of(identity);
        let choice = |title: &str, subtitle: &str| Row {
            id: title.into(),
            title: title.into(),
            subtitle: Some(subtitle.into()),
            unavailable: None,
        };
        state.next_screen();
        state.entries = vec![Entry::ClearCache(identity.clone()), Entry::Cancel];
        let details = vec![
            format!("From {identity}"),
            "Pane deletes the data this extension keeps as its cache. Its settings, content and \
             credentials are kept, and the extension does not run."
                .into(),
        ];
        let screen = Screen::Confirm {
            question: Question::ClearCache(identity.clone()),
            details,
        };
        state.view =
            LauncherView::new(screen, format!("Clear the cache of {title}?")).with_rows(vec![
                choice("Clear cache", "Delete the cached data now"),
                choice("Cancel", "Keep the cache"),
            ]);
    }

    /// Clears the cache of the installed package with `identity` without
    /// running it, then shows the extension list with the outcome. An
    /// instance of it that is running keeps what it holds in memory and may
    /// save it to its cache again, which the outcome then says.
    async fn clear_cache(&self, epoch: u64, identity: PackageIdentity) {
        let cleared = match &self.installation {
            Some(installation) => {
                let data = installation.data.clone();
                let identity = identity.clone();
                off_thread(move || data.clear_cache(&identity)).await
            }
            None => Err("this launcher does not install packages".into()),
        };
        let components: Vec<PathBuf> = {
            let state = self.lock();
            let package = state.package(&identity);
            package.map_or_else(Vec::new, |package| {
                package
                    .commands()
                    .into_iter()
                    .map(|c| c.component)
                    .collect()
            })
        };
        let running = match self.runtime() {
            Ok(runtime) => runtime.running().await,
            Err(_) => Vec::new(),
        };
        let still_running = running.iter().any(|path| components.contains(path));
        let Some(mut state) = self.lock_if_current(epoch) else {
            return;
        };
        let title = state.title_of(&identity);
        self.show_extensions_at_clear_cache(&mut state, &identity);
        state.view.status = match cleared {
            Ok(()) if still_running => Status::Result(format!(
                "Cleared the cache of {title}. A running instance may write it again until it \
                 stops."
            )),
            Ok(()) => Status::Result(format!("Cleared the cache of {title}")),
            Err(reason) => Status::Error(format!("Could not clear the cache of {title}: {reason}")),
        };
    }

    /// Returns from a confirmation to the extension list without acting.
    fn leave_confirm(&self, state: &mut State) {
        let Screen::Confirm { question, .. } = &state.view.screen else {
            return;
        };
        match question.clone() {
            Question::ClearCache(identity) => self.show_extensions_at_clear_cache(state, &identity),
            Question::Uninstall(identity) | Question::UninstallDependents(identity) => self
                .show_extensions_at(
                    state,
                    |entry| matches!(entry, Entry::AskUninstall(asked) if *asked == identity),
                ),
            Question::DeleteRetained(identity) => self.show_extensions_at(
                state,
                |entry| matches!(entry, Entry::AskDeleteRetained(asked) if *asked == identity),
            ),
            Question::DisableDependents(identity) => self.show_extensions_at(
                state,
                |entry| matches!(entry, Entry::Toggle(asked) if *asked == identity),
            ),
        }
    }

    /// Shows the extension list with the row that clears the cache of the
    /// package with `identity` selected, where the user asked.
    fn show_extensions_at_clear_cache(&self, state: &mut State, identity: &PackageIdentity) {
        self.show_extensions_at(
            state,
            |entry| matches!(entry, Entry::AskClearCache(asked) if asked == identity),
        );
    }

    /// Shows the extension list with the first row whose entry is `wanted`
    /// selected, or the first row if there is none.
    fn show_extensions_at(&self, state: &mut State, wanted: impl Fn(&Entry) -> bool) {
        self.show_extensions(state);
        let row = state.entries.iter().position(wanted);
        if row.is_some() {
            state.view.selected = row;
        }
    }

    /// Updates the installed packages on screen after one changed, keeping
    /// the selection on the same row.
    fn refresh_extensions(&self, state: &mut State) {
        let (rows, entries) = self.extension_rows(state);
        // The same row stays selected; if it is gone (a Retry row once the
        // package started), the row before it.
        let selected = state.view.selected.and_then(|index| {
            let id = &state.view.rows.get(index)?.id;
            rows.iter()
                .position(|row| row.id == *id)
                .or_else(|| Some(index.saturating_sub(1).min(rows.len().checked_sub(1)?)))
        });
        state.entries = entries;
        state.view.selected = selected.or_else(|| first_index(&rows));
        state.view.rows = rows;
    }

    /// Opens the custom view of `item_id` and shows its first drawing, or
    /// closes it again if the user has left the command meanwhile.
    async fn open_custom_view(
        &self,
        epoch: u64,
        component: PathBuf,
        item_id: String,
        info: CustomViewInfo,
        data: Option<PackageData>,
    ) {
        let result = match self.runtime() {
            Ok(runtime) => {
                runtime
                    .open_view_with(&component, &item_id, data.clone())
                    .await
            }
            Err(error) => Err(error),
        };
        let current = self.lock_if_current(epoch);
        let stopped = current
            .as_ref()
            .and_then(|state| stopped(state, &component, &data));
        let Some(mut state) = current.filter(|_| stopped.is_none()) else {
            if let (Ok((id, _)), Ok(runtime)) = (result, self.runtime()) {
                runtime.close_view(id);
            }
            if let Some(problem) = stopped {
                // Stopped while it was opening.
                self.lock().view.status = Status::Error(problem);
            }
            return;
        };
        match result {
            Ok((id, frame)) => {
                let snapshot = CustomViewSnapshot {
                    id,
                    label: info.label,
                    role: info.role,
                    frame,
                };
                let view = LauncherView::new(Screen::CustomView(snapshot), info.title);
                let return_to = LauncherView {
                    status: Status::Idle,
                    ..std::mem::replace(&mut state.view, view)
                };
                state.custom_view = Some(OpenCustomView {
                    id,
                    return_to,
                    pressed: false,
                    sent: 0,
                    shown: 0,
                    moves_in_flight: 0,
                    waiting_move: None,
                });
                state.next_screen();
            }
            Err(error) => state.view.status = Status::Error(error.to_string()),
        }
    }

    /// Sends the user's input to the open custom view. Await the returned
    /// future to show the view's new drawing. A pointer move or release is
    /// sent only while the button pressed over the view is held; anything
    /// sent when no view is open is ignored.
    ///
    /// Events are handled in order, and a drawing is shown only if no later
    /// event's drawing is on screen yet. An error the extension reports is
    /// shown while the view stays open; a crash closes the view.
    ///
    /// A drag is coalesced: while a pointer move is being handled, a further
    /// move is not sent; only the latest one waiting is, once the moves in
    /// flight are answered (by the future of the move answered last), or
    /// before the next other event. Await every returned future.
    pub fn send_view_event(&self, event: ViewEvent) -> impl Future<Output = ()> + Send + 'static {
        let mut state = self.lock();
        let epoch = state.screen_epoch;
        // Sent now, so the view handles events in the order of these calls
        // whenever the returned futures are awaited.
        let mut sent: VecDeque<SentEvent> = self.send_to_view(&mut state, event).into();
        drop(state);
        let launcher = self.clone();
        async move {
            while let Some(event) = sent.pop_front() {
                let result = event.reply.await;
                launcher.show_view_answer(epoch, event.number, result);
                if event.is_move {
                    sent.extend(launcher.finish_move(epoch));
                }
            }
        }
    }

    /// Whether the primary pointer button was pressed over the open view and
    /// is still held, so the window should forward pointer moves and the
    /// release.
    pub fn pointer_held(&self) -> bool {
        self.lock()
            .custom_view
            .as_ref()
            .is_some_and(|open| open.pressed)
    }

    /// Sends `event` to the open view, after a waiting move, unless it is a
    /// move or release with no press held, or a move to wait (see
    /// [`Launcher::send_view_event`]).
    fn send_to_view(&self, state: &mut State, event: ViewEvent) -> Vec<SentEvent> {
        let (Some(open), Ok(runtime)) = (state.custom_view.as_mut(), self.runtime()) else {
            return Vec::new();
        };
        match event {
            ViewEvent::PointerDown(_) => open.pressed = true,
            ViewEvent::PointerMove(_) | ViewEvent::PointerUp(_) if !open.pressed => {
                return Vec::new();
            }
            ViewEvent::PointerUp(_) => open.pressed = false,
            ViewEvent::PointerMove(_) | ViewEvent::Key(_) => {}
        }
        if let ViewEvent::PointerMove(at) = event
            && open.moves_in_flight > 0
        {
            open.waiting_move = Some(at);
            return Vec::new();
        }
        let mut sent = Vec::new();
        if let Some(at) = open.waiting_move.take() {
            sent.push(open.send(runtime, ViewEvent::PointerMove(at)));
        }
        sent.push(open.send(runtime, event));
        sent
    }

    /// Notes that a move sent to the view of `epoch` was answered, and
    /// sends the waiting move once no other move is in flight.
    fn finish_move(&self, epoch: u64) -> Option<SentEvent> {
        let mut state = self.lock_if_current(epoch)?;
        let runtime = self.runtime().ok()?;
        let open = state.custom_view.as_mut()?;
        open.moves_in_flight = open.moves_in_flight.saturating_sub(1);
        if open.moves_in_flight > 0 {
            return None;
        }
        let at = open.waiting_move.take()?;
        Some(open.send(runtime, ViewEvent::PointerMove(at)))
    }

    /// Shows the open view's answer to its event number `number`.
    fn show_view_answer(&self, epoch: u64, number: u64, result: Result<Frame, CallError>) {
        let Some(mut state) = self.lock_if_current(epoch) else {
            return;
        };
        let state = &mut *state;
        let open = state.custom_view.as_mut().expect("a view is open");
        match result {
            // An answer to an event older than the one on screen is stale.
            Ok(_) | Err(CallError::Guest(_)) if number <= open.shown => {}
            Ok(frame) => {
                open.shown = number;
                let Screen::CustomView(snapshot) = &mut state.view.screen else {
                    unreachable!("a view is open");
                };
                snapshot.frame = frame;
                state.view.status = Status::Idle;
            }
            // The view refused the event and keeps its drawing.
            Err(error @ CallError::Guest(_)) => {
                open.shown = number;
                state.view.status = Status::Error(error.to_string());
            }
            // The guest instance, and the view with it, is gone.
            Err(error) => self.return_from_custom_view(state, Status::Error(error.to_string())),
        }
    }

    /// Closes the open custom view and shows the command view it was opened
    /// from, with `status`, as a new screen.
    fn return_from_custom_view(&self, state: &mut State, status: Status) {
        let return_to = self.close_custom_view(state).expect("a view is open");
        state.next_screen();
        state.view = LauncherView {
            status,
            ..return_to
        };
    }

    /// Leaves the open command, and any form or custom view of it, for
    /// another screen, which the caller then shows: the view is closed in
    /// the runtime, and replies for the old screen are discarded.
    fn leave_command(&self, state: &mut State) {
        self.close_custom_view(state);
        // Its search in progress, if any, is stopped.
        state.searching = None;
        state.open = None;
        state.form = None;
        state.next_screen();
    }

    /// Closes the open custom view, if there is one, and returns the command
    /// view it was opened from.
    fn close_custom_view(&self, state: &mut State) -> Option<LauncherView> {
        let open = state.custom_view.take()?;
        if let Ok(runtime) = self.runtime() {
            runtime.close_view(open.id);
        }
        Some(open.return_to)
    }

    /// Opens `url` with the link opener, off the calling thread, and reports
    /// the outcome while the screen is the one it was opened from.
    async fn open_url(&self, epoch: u64, url: String) {
        let links = self.links.clone();
        let opened = {
            let url = url.clone();
            off_thread(move || links.open(&url)).await
        };
        let Some(mut state) = self.lock_if_current(epoch) else {
            return;
        };
        state.view.status = match opened {
            Ok(()) => Status::Result(format!("Opened {url}")),
            Err(reason) => Status::Error(format!("Could not open {url}: {reason}")),
        };
    }

    /// Sends the query of `sending` to its command, which takes a query,
    /// and shows its answer while root search still shows the query it was
    /// sent from; root search stays as it was. The answer to a query the
    /// user has changed since is not shown.
    async fn run_query(&self, epoch: u64, sending: aliases::Sending, data: Option<PackageData>) {
        let aliases::Sending {
            component,
            command,
            query,
            ..
        } = sending;
        let result = match self.runtime() {
            Ok(runtime) => {
                runtime
                    .run_query_with(&component, &command, &query, data.clone())
                    .await
            }
            Err(error) => Err(error),
        };
        let Some(mut state) = self.lock_if_current(epoch) else {
            return;
        };
        let Some(sent) = state.sent_from.clone() else {
            // The query changed meanwhile, which cleared the status.
            return;
        };
        if state.view.query() != Some(sent.as_str()) {
            return;
        }
        state.view.status = match (stopped(&state, &component, &data), result) {
            // Stopped while it was running: its answer is not shown.
            (Some(problem), _) => Status::Error(problem),
            (None, Ok(answer)) => Status::Result(answer),
            (None, Err(error)) => Status::Error(error.to_string()),
        };
    }

    async fn run_action(
        &self,
        epoch: u64,
        component: PathBuf,
        item_id: String,
        data: Option<PackageData>,
    ) {
        let result = match self.runtime() {
            Ok(runtime) => {
                runtime
                    .run_action_with(&component, &item_id, data.clone())
                    .await
            }
            Err(error) => Err(error),
        };
        let Some(mut state) = self.lock_if_current(epoch) else {
            return;
        };
        state.view.status = match (stopped(&state, &component, &data), result) {
            // Stopped while it was running: its answer is not shown.
            (Some(problem), _) => Status::Error(problem),
            (None, Ok(answer)) => Status::Result(answer),
            (None, Err(error)) => Status::Error(error.to_string()),
        };
    }

    async fn open_command(&self, epoch: u64, opening: Opening, data: Option<PackageData>) {
        let Opening {
            component,
            command,
            search,
        } = opening;
        let result = match self.runtime() {
            Ok(runtime) => runtime.get_view_with(&component, data.clone()).await,
            Err(error) => Err(error),
        };
        let Some(mut state) = self.lock_if_current(epoch) else {
            return;
        };
        let end = data.as_ref().and_then(PackageData::stopped);
        if end == Some(End::Disabled) {
            // Disabled while it was opening.
            state.view.status = Status::Error(disabled(&state, &component));
            return;
        }
        if end == Some(End::Replaced) {
            // Reloaded or updated while it was opening: the call was stopped,
            // or its answer came from code that no longer runs. Its package's commands
            // are in root search again. Unless the reload or update has
            // reported its outcome meanwhile, this opening is still shown as
            // running, so it ends here.
            if state.view.status == Status::Running {
                state.view.status = Status::Error(
                    "The extension changed while its command was opening; open it again".into(),
                );
            }
            return;
        }
        if end == Some(End::Uninstalled) {
            // Uninstalled while it was opening: the uninstall reports its
            // own outcome.
            return;
        }
        if end == Some(End::Paused) {
            // Paused before it was asked (by its hotkey), or while it was
            // opening (this opening crashed or could not start, which said
            // so).
            if state.view.status == Status::Running {
                state.view.status = Status::Error(paused(&state, &component));
            }
            return;
        }
        let state = &mut *state;
        match result {
            Ok(view) => {
                let CommandList {
                    mut rows,
                    mut entries,
                } = CommandList::of(view.items);
                if let Some(package) = owner(&state.packages, &component)
                    && folder_access(package)
                {
                    let (pane_rows, pane_entries) = files::folder_rows(state, &package.identity);
                    rows.splice(0..0, pane_rows);
                    entries.splice(0..0, pane_entries);
                }
                let screen = if search {
                    state.searching = Some(command_search::Searching::new(command));
                    Screen::CommandSearch {
                        query: String::new(),
                    }
                } else {
                    state.searching = None;
                    Screen::Command
                };
                state.entries = entries;
                state.open = Some(component);
                state.next_screen();
                state.view = LauncherView::new(screen, view.title).with_rows(rows);
            }
            Err(error) => state.view.status = Status::Error(error.to_string()),
        }
    }

    /// The extension data of the installed package `component` belongs to,
    /// in its current generation; `None` for a command built into Pane.
    fn data_of(&self, component: &Path) -> Option<PackageData> {
        self.data_in(&self.lock(), component)
    }

    /// Like [`Launcher::data_of`], with the state locked.
    fn data_in(&self, state: &State, component: &Path) -> Option<PackageData> {
        let package = owner(&state.packages, component)?;
        Some(self.installation.as_ref()?.data.owned_by(&package.identity))
    }

    /// What an update or reload of the package with `identity` does to the
    /// old code once the new copy is recorded, before the old copy's folder
    /// is removed: its generation ends, which stops its pending calls (the
    /// new code runs in a new one), and its helpers are ended and reaped, so
    /// no running program keeps the folder in use (Windows would refuse to
    /// remove it).
    fn retire(&self, identity: &PackageIdentity) -> impl FnOnce(&Path) + Send + 'static {
        let data = self.installation.as_ref().map(|i| i.data.clone());
        let runtime = self.runtime().ok().cloned();
        let identity = identity.clone();
        move |old: &Path| {
            if let Some(data) = data {
                data.replace_code(&identity);
            }
            if let Some(runtime) = runtime {
                runtime.stop_helpers_in(old);
            }
        }
    }

    fn runtime(&self) -> Result<&Runtime, CallError> {
        self.runtime.as_ref().map_err(Clone::clone)
    }

    /// Locks the state only if the screen is still the one of `epoch`.
    fn lock_if_current(&self, epoch: u64) -> Option<MutexGuard<'_, State>> {
        let state = self.lock();
        (state.screen_epoch == epoch).then_some(state)
    }

    /// Locks the launcher's state. If a thread panicked while holding it
    /// (such as the runtime thread crashing while it noted a failure), the
    /// state is taken over and first put back into a known state (see
    /// [`Launcher::recover_state`]).
    fn lock(&self) -> MutexGuard<'_, State> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                self.state.clear_poison();
                self.recover_state(&mut state);
                state
            }
        }
    }

    /// Tells the window that the launcher changed in the background.
    fn changed(&self) {
        if let Some(changes) = &self.changes {
            changes.changed();
        }
    }
}

/// The installed package whose managed copy holds `component`.
fn owner<'a>(packages: &'a [InstalledPackage], component: &Path) -> Option<&'a InstalledPackage> {
    packages
        .iter()
        .find(|package| component.starts_with(&package.location))
}

/// Why the answer of a call into `component` made with `data` is not shown:
/// the generation it belonged to has ended, since its package was disabled
/// ("<title> is disabled"), its code replaced, it was uninstalled or Pane
/// paused it. `None` while it lasts, and for a command built into Pane.
fn stopped(state: &State, component: &Path, data: &Option<PackageData>) -> Option<String> {
    match data.as_ref()?.stopped()? {
        End::Disabled => Some(disabled(state, component)),
        End::Replaced => Some(CallError::Replaced.to_string()),
        End::Uninstalled => Some(CallError::Uninstalled.to_string()),
        End::Paused => Some(paused(state, component)),
    }
}

/// "<title> is paused after an error; …", for the package `component`
/// belongs to.
fn paused(state: &State, component: &Path) -> String {
    match owner(&state.packages, component) {
        Some(package) => paused_reason(&package.title()),
        None => CallError::Paused.to_string(),
    }
}

/// "<title> is disabled", for the package `component` belongs to.
fn disabled(state: &State, component: &Path) -> String {
    match owner(&state.packages, component) {
        Some(package) => format!("{} is disabled", package.title()),
        None => CallError::Disabled.to_string(),
    }
}

/// One row per installed package, saying whether it is enabled or paused
/// and which source it is, so copies with the same title can be told apart;
/// then the rows that reload each enabled package, each followed, if Pane
/// paused it, by a row that retries it and one that shows why it is paused;
/// then one row per package to clear its cache, and one to uninstall it, in
/// the same order.
fn extension_rows(
    packages: &[InstalledPackage],
    paused: &Pauses,
    developed: impl Fn(&PackageIdentity) -> bool,
) -> (Vec<Row>, Vec<Entry>) {
    let failure = |package: &InstalledPackage| paused.of(&package.identity).cloned();
    let toggles = packages.iter().map(|package| {
        let state = match (package.enabled, failure(package).map(|pause| pause.after)) {
            (false, _) => "Disabled",
            (true, None) => "Enabled",
            (true, Some(PauseCause::FailedToStart)) => "Enabled · Failed to start",
            (true, Some(PauseCause::Crashes)) => "Enabled · Paused after crashing",
        };
        let developing = if developed(&package.identity) {
            " · Developing"
        } else {
            ""
        };
        let row = Row {
            id: package.identity.key(),
            title: package.title(),
            subtitle: Some(format!(
                "{state}{developing}{network} · {}",
                package.identity,
                network = if package.uses_network {
                    format!(" · {}", network::USES_THE_NETWORK)
                } else {
                    String::new()
                }
            )),
            unavailable: None,
        };
        (row, Entry::Toggle(package.identity.clone()))
    });
    let reloads = packages
        .iter()
        .filter(|package| package.enabled)
        .flat_map(|package| {
            let title = package.title();
            let source = match package.identity.local_folder() {
                Some(folder) => folder.display().to_string(),
                None => package.identity.to_string(),
            };
            let reload = Row {
                id: format!("reload:{}", package.identity.key()),
                title: format!("Reload {title}"),
                subtitle: Some(format!(
                    "Replace its code with the current build in {source}"
                )),
                unavailable: None,
            };
            let paused = failure(package).into_iter().flat_map(move |pause| {
                let retry = Row {
                    id: format!("retry:{}", package.identity.key()),
                    title: pausing::retry_title(&title, pause.after),
                    subtitle: Some(format!(
                        "Paused: {}; start it again",
                        pausing::failure("it", pause.after)
                    )),
                    unavailable: None,
                };
                let details = Row {
                    id: format!("paused:{}", package.identity.key()),
                    title: pausing::details_title(&title),
                    subtitle: Some("The error and its diagnostics".into()),
                    unavailable: None,
                };
                [
                    (retry, Entry::Retry(package.identity.clone())),
                    (details, Entry::PauseDetails(package.identity.clone())),
                ]
            });
            // A package from npm has no source folder to reload from; to
            // replace its code, install it from npm again (Update).
            let local = package.identity.local_folder().is_some();
            std::iter::once((reload, Entry::Reload(package.identity.clone())))
                .filter(move |_| local)
                .chain(paused)
        });
    let clear_cache = packages.iter().map(|package| {
        let row = Row {
            id: format!("clear-cache:{}", package.identity.key()),
            title: format!("Clear cache of {}", package.title()),
            subtitle: Some(format!(
                "Keeps its settings, content and credentials · {}",
                package.identity
            )),
            unavailable: None,
        };
        (row, Entry::AskClearCache(package.identity.clone()))
    });
    let uninstall = packages.iter().map(|package| {
        let row = Row {
            id: format!("uninstall:{}", package.identity.key()),
            title: format!("Uninstall {}", package.title()),
            subtitle: Some(format!(
                "Remove it and choose whether to keep its saved data · {}",
                package.identity
            )),
            unavailable: None,
        };
        (row, Entry::AskUninstall(package.identity.clone()))
    });
    toggles
        .chain(reloads)
        .chain(clear_cache)
        .chain(uninstall)
        .unzip()
}

/// The package screen for `request`: what the package is and whether it
/// can be installed, or why it cannot.
fn preview_view(
    request: &install::Request,
    checked: Result<(SourcePackage, dependencies::Plan), PackageError>,
    installed: Option<InstalledPackage>,
) -> (LauncherView, Vec<Entry>) {
    let (package, plan) = match checked {
        Ok(checked) => checked,
        Err(error) => {
            let (asked, name) = request.describe();
            let view = LauncherView {
                status: Status::Error(error.to_string()),
                ..LauncherView::new(
                    Screen::Package {
                        details: vec![asked],
                    },
                    format!("Cannot install {name}"),
                )
            };
            return (view, Vec::new());
        }
    };
    let manifest = &package.manifest;
    let mut details = vec![format!("Source: {}", package.identity)];
    if let Some(version) = &manifest.version {
        details.push(format!("Version: {version}"));
    }
    if let Some(npm) = &package.npm {
        let pinned_to = installed
            .as_ref()
            .and_then(|installed| installed.npm.as_ref())
            .filter(|installed| installed.pinned)
            .map(|installed| installed.version.as_str());
        details.extend(npm_lines(npm, pinned_to));
    }
    let titles: Vec<&str> = manifest.commands.iter().map(|c| c.title.as_str()).collect();
    if !titles.is_empty() {
        details.push(format!("Commands: {}", titles.join(", ")));
    }
    if let Some(operations) = operations::describe(&manifest.operations) {
        details.push(operations);
    }
    if let Some(helpers) = helpers::describe(&manifest.helpers) {
        details.push(helpers);
    }
    details.push(format!(
        "Compatible: needs extension API {}, and its components import only WASI 0.3",
        manifest.api_version
    ));
    if let Some(platforms) = &manifest.platforms {
        // A package that does not support this system is explained instead.
        let names: Vec<String> = platforms
            .iter()
            .map(|&platform| {
                if Some(platform) == Platform::current() {
                    format!("{platform} (this system)")
                } else {
                    platform.to_string()
                }
            })
            .collect();
        details.push(format!("Supported systems: {}", platform::join(&names)));
    }
    details.extend(plan.lines());
    if !plan.problems.is_empty() {
        // Nothing is offered: a required dependency cannot be installed.
        let view = LauncherView {
            status: Status::Error(install::problems(&plan).to_string()),
            ..LauncherView::new(
                Screen::Package { details },
                format!("Cannot install {}", manifest.title),
            )
        };
        return (view, Vec::new());
    }
    let with = match plan.installed_with().as_slice() {
        [] => String::new(),
        [one] => format!(", and install {one}, which it requires"),
        titles => format!(", and install the {} extensions it requires", titles.len()),
    };
    let (row, entry) = match installed {
        Some(installed) => {
            details.push(match (&installed.npm, installed.version()) {
                (Some(npm), _) => format!(
                    "Installed: npm version {}{} of this package",
                    npm.version,
                    if npm.pinned { ", pinned" } else { "" }
                ),
                (None, Some(version)) => format!("Installed: version {version} from this folder"),
                (None, None) => "Installed from this folder".into(),
            });
            if !installed.enabled {
                details.push("Disabled: enable it in Manage extensions".into());
            }
            let replace = match package.npm.as_ref().map(|npm| &npm.package) {
                Some(npm) if npm.pinned => format!("npm version {}, pinned", npm.version),
                Some(npm) => format!("npm version {}, the latest", npm.version),
                None => "this folder's contents".into(),
            };
            let row = Row {
                id: "update".into(),
                title: "Update".into(),
                subtitle: Some(format!("Replace the installed copy with {replace}{with}")),
                unavailable: None,
            };
            let mode = Mode::Update(installed.identity.clone());
            (
                row,
                Entry::Install(request.clone(), mode, plan.assumptions.clone()),
            )
        }
        None => {
            let row = Row {
                id: "install".into(),
                title: "Install".into(),
                subtitle: Some(format!(
                    "Copy the package into Pane and add its commands{with}"
                )),
                unavailable: None,
            };
            (
                row,
                Entry::Install(request.clone(), Mode::Install, plan.assumptions.clone()),
            )
        }
    };
    let view =
        LauncherView::new(Screen::Package { details }, manifest.title.clone()).with_rows(vec![row]);
    (view, vec![entry])
}

/// The preview's lines about where a package from npm was downloaded from,
/// and what Pane does not do with it; `pinned_to` is the version the
/// installed copy is pinned to, if it is.
fn npm_lines(npm: &crate::npm::NpmOrigin, pinned_to: Option<&str>) -> Vec<String> {
    let version = &npm.package.version;
    let mut lines = vec![
        if npm.package.pinned && pinned_to == Some(version) {
            format!(
                "npm version: {version}, the version it is pinned to: name another version to \
                 change it"
            )
        } else if npm.package.pinned {
            format!(
                "npm version: {}, the version you named: installing pins it to that version",
                npm.package.version
            )
        } else {
            format!("npm version: {}, the latest", npm.package.version)
        },
        format!(
            "Downloaded: {}, matching its sha512 integrity from the registry",
            npm.tarball
        ),
        "Runs only the WebAssembly components its pane.json names, in Pane: no Node.js, npm \
         install scripts or npm dependencies"
            .into(),
    ];
    if !npm.scripts.is_empty() || npm.has_npm_dependencies {
        let mut ignored = Vec::new();
        if !npm.scripts.is_empty() {
            let scripts: Vec<String> = npm.scripts.iter().map(|s| format!("`{s}`")).collect();
            ignored.push(format!("its {} script", platform::join(&scripts)));
        }
        if npm.has_npm_dependencies {
            ignored.push("its npm dependencies".into());
        }
        lines.push(format!(
            "Not used: {}, which its package.json declares; Pane never runs or installs them",
            platform::join(&ignored)
        ));
    }
    lines
}

/// Replaces the command view with `form`, which belongs to item `item_id`.
/// The command's row entries stay, for when the form closes.
fn open_form(state: &mut State, item_id: String, form: Form) {
    let fields = form
        .fields
        .into_iter()
        .map(|field| {
            let value = match &field.kind {
                FieldKind::Text { .. } => String::new(),
                FieldKind::Choice(choices) => choices
                    .first()
                    .map(|choice| choice.id.clone())
                    .unwrap_or_default(),
            };
            FormField {
                id: field.id,
                label: field.label,
                kind: field.kind,
                value,
                error: None,
            }
        })
        .collect();
    let form_view = LauncherView::new(
        Screen::Form(FormView {
            fields,
            submit_label: form.submit_label,
        }),
        form.title,
    );
    let return_to = std::mem::replace(&mut state.view, form_view);
    state.form = Some(OpenForm {
        purpose: FormPurpose::Item(item_id),
        return_to,
        submitting: false,
    });
    state.next_screen();
}

/// The rows of root search for `query`, and what activating each does: the
/// results computed from it, then the root results matching it, best match
/// first, with, for a query that is not blank, those supplied ahead of it
/// (after the others of the same rank), then the computed results that open
/// a file, then the rows explaining why a command could not supply them.
fn root_rows(state: &State, query: &str) -> (Vec<Row>, Vec<Entry>) {
    let blank = query.trim().is_empty();
    let candidates: Vec<&RootResult> = state
        .root
        .iter()
        .chain(state.indexes.results().filter(|_| !blank))
        .collect();
    let keys: Vec<&Keys> = candidates.iter().map(|result| &result.keys).collect();
    let parsed = Query::new(query);
    let named = |index: &usize| parsed.is_alias_of(&candidates[*index].keys);
    let by_alias: Vec<usize> = (0..candidates.len()).filter(named).collect();
    let matches: Vec<usize> = search::ranked_matches(&parsed, keys.into_iter())
        .into_iter()
        .filter(|index| !named(index))
        .collect();
    let found = |index: usize| {
        (
            candidates[index].row.clone(),
            candidates[index].entry.clone(),
        )
    };
    let failures = state
        .indexes
        .failures()
        .filter(|_| !blank)
        .map(|(row, entry)| (row.clone(), entry.clone()));
    let (files, computed): (Vec<&Computed>, Vec<&Computed>) = state
        .computed
        .iter()
        .partition(|computed| matches!(computed.entry, Entry::OpenFile { .. }));
    let computed_row = |computed: &Computed| (computed.row.clone(), computed.entry.clone());
    // What the user's alias names comes first, even before computed
    // results; files found for the query follow what is found by title,
    // since a folder can hold many; the fallbacks, which the user must
    // choose, come last.
    aliases::rows_sending_after_alias(state, query)
        .into_iter()
        .chain(by_alias.into_iter().map(found))
        .chain(computed.into_iter().map(computed_row))
        .chain(matches.into_iter().map(found))
        .chain(files.into_iter().map(computed_row))
        .chain(failures)
        .chain(aliases::fallback_rows(state, query))
        .unzip()
}

/// Lists root search's rows for `query` again after results arrived: the
/// best match stays selected, now that better ones may be first, and a row
/// the user moved to stays selected.
fn relist_root(state: &mut State, query: &str) {
    let keep = state
        .view
        .selected
        .filter(|&index| index > 0)
        .and_then(|index| state.view.rows.get(index))
        .map(|row| row.id.clone());
    let (rows, entries) = root_rows(state, query);
    state.view.selected = keep
        .and_then(|id| rows.iter().position(|row| row.id == id))
        .or_else(|| aliases::first_choice(&entries));
    state.view.rows = rows;
    state.entries = entries;
}

/// The rows for `command`'s `answer` to `query`: its results, or one
/// explaining why it failed. A result that opens a file is shown with the
/// file's own name and folder, as the host found it in the latest listing
/// of `owner`'s granted folder, whatever the extension titled it; one the
/// host does not know is left out.
fn computed_results(
    command: CommandRegistration,
    owner: Option<&str>,
    files: Option<&FileAccess>,
    query: &str,
    answer: Result<Vec<ComputedResult>, CallError>,
) -> Vec<Computed> {
    let computed = |row: Row, entry: Entry| Computed {
        component: command.component.clone(),
        row,
        entry,
    };
    match answer {
        Ok(results) => results
            .into_iter()
            .filter_map(|result| {
                let ComputedResult { listing, action } = result;
                let (listing, entry) = match action {
                    RootAction::Copy(text) => (listing, Entry::Copy(text)),
                    RootAction::OpenUrl(url) => (listing, Entry::OpenUrl(url)),
                    RootAction::OpenFile(id) => {
                        let owner = owner?;
                        let known = files?.known(owner, &id)?;
                        let entry = Entry::OpenFile {
                            owner: owner.to_owned(),
                            id,
                            name: known.name.clone(),
                        };
                        let listing = ResultListing {
                            id: listing.id,
                            title: known.name.clone(),
                            subtitle: Some(format!("File in {}", known.within)),
                        };
                        (listing, entry)
                    }
                };
                Some(computed(Row::listed(listing, Some(&command.id)), entry))
            })
            .collect(),
        Err(error) => {
            let row = Row {
                id: format!("{}:failed", command.id),
                title: command.title.clone(),
                subtitle: Some(format!("Could not answer: {error}")),
                unavailable: None,
            };
            let problem = format!("{} could not answer “{query}”: {error}", command.title);
            vec![computed(row, Entry::Broken(problem))]
        }
    }
}

/// What resolves once a granted folder's listing ends.
type ListedFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A command whose answer waited for its granted folder's listing, its
/// data, and what resolves once the listing ends.
type Listing = (CommandRegistration, Option<PackageData>, ListedFuture);

/// Whether `package` asks for access to a folder the user grants it.
fn folder_access(package: &InstalledPackage) -> bool {
    package
        .manifest
        .as_ref()
        .is_ok_and(|manifest| manifest.folder_access)
}

/// Awaits `call`, unless `cancelled` resolves first (its sender was used or
/// dropped): then `call` is dropped unfinished, and the answer is `None`.
async fn until_cancelled<T>(
    call: impl Future<Output = T>,
    cancelled: &mut tokio::sync::oneshot::Receiver<()>,
) -> Option<T> {
    let mut call = std::pin::pin!(call);
    std::future::poll_fn(|cx| {
        if Pin::new(&mut *cancelled).poll(cx).is_ready() {
            return std::task::Poll::Ready(None);
        }
        call.as_mut().poll(cx).map(Some)
    })
    .await
}

/// Runs blocking file work on its own thread, so the caller's thread (the
/// window's) never waits on the file system.
async fn off_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    let (reply, response) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = reply.send(work());
    });
    response.await.expect("package file work panicked")
}

fn first_index(rows: &[Row]) -> Option<usize> {
    (!rows.is_empty()).then_some(0)
}
