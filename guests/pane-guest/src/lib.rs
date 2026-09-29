//! Guest-side bindings for Pane's `pane:extension` contract.
//!
//! An extension implements [`Guest`] and calls [`export!`]. It may keep
//! values between runs with [`settings`], and its own records, disposable
//! values and secrets with [`content`], [`cache`] and [`credentials`]. It
//! may compute results from root search's query with [`root`], take a query
//! the user sends it from root search with [`query`], call
//! operations other packages publish with [`operations::call`], serve those
//! its own package publishes with [`publish`], find and open installed
//! applications with [`applications`], supply root results ahead of the
//! query with [`indexed`], run its package's native helpers with
//! [`helpers`], list the files of a folder with [`files`], search as the
//! user types into its own search field with [`search`], make web
//! requests with [`http`] and keep clipboard history with
//! [`clipboard_history`]. The crate is
//! `no_std` so the component imports only WASI 0.3 interfaces; it supplies the
//! allocator and a panic handler that traps, which the host reports as a
//! runtime error.
//!
//! Without `std` no libc is linked, so the crate also supplies what the
//! compiler and the component runtime call into libc or `std` for:
//! `memcmp` and `bcmp`, which the compiler emits for byte and string
//! comparisons (`==` on `str`, `starts_with`, ...) as soon as a guest compares
//! strings, and the canonical-ABI `cabi_realloc`.
#![no_std]

pub extern crate alloc;

use core::ffi::c_void;

wit_bindgen::generate!({
    path: "../../wit",
    world: "extension-with-data",
    pub_export_macro: true,
    default_bindings_module: "pane_guest",
});

pub use exports::pane::extension::command::{
    Choice, CustomView, CustomViewInfo, CustomViewRole, Field, FieldKind, FieldValue, Form,
    FormError, Frame, Guest, GuestCustomView, Item, Key, Platform, Point, Rect, Shape, Text,
    TextField, View, ViewEvent,
};
pub use pane::extension::{cache, content, credentials, operations, settings};

impl operations::CallErrorKind {
    /// The kind's WIT name, such as `not-found`, as JavaScript sees it too.
    pub fn name(&self) -> &'static str {
        use operations::CallErrorKind::*;
        match self {
            NotFound => "not-found",
            Disabled => "disabled",
            Incompatible => "incompatible",
            Unavailable => "unavailable",
            Failed => "failed",
            Crashed => "crashed",
            Refused => "refused",
        }
    }
}

impl operations::CallError {
    /// `<kind>: <message>`, such as "failed: a name is needed", to show
    /// people.
    pub fn explain(&self) -> alloc::string::String {
        alloc::format!("{}: {}", self.kind.name(), self.message)
    }
}

/// Serving the operations a package publishes
/// (`pane:extension/published-operations`). The component its `pane.json`
/// names under `operations` implements [`publish::Guest`] too and calls
/// [`publish::export!`](crate::publish::export) beside [`export!`]:
///
/// ```ignore
/// pane_guest::export!(Greeter);
/// pane_guest::publish::export!(Greeter);
/// ```
pub mod publish {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "operations-provider",
        pub_export_macro: true,
        default_bindings_module: "pane_guest::publish",
    });

    pub use exports::pane::extension::published_operations::Guest;
}

/// Results a command computes from root search's query
/// (`pane:extension/root-results`), such as a calculator's answer. A command
/// whose `pane.json` entry sets `"rootResults": true` implements
/// [`root::Guest`] too and calls [`root::export!`](crate::root::export)
/// beside [`export!`]:
///
/// ```ignore
/// pane_guest::export!(Calculator);
/// pane_guest::root::export!(Calculator);
/// ```
pub mod root {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "root-results-provider",
        pub_export_macro: true,
        default_bindings_module: "pane_guest::root",
    });

    pub use exports::pane::extension::root_results::{Guest, RootAction, RootResult};
}

/// A command that takes a query (`pane:extension/query-command`): text the
/// user typed into root search, which Pane sends only when the user invokes
/// the command through its alias ("ec hello") or chooses it as a fallback.
/// A command whose `pane.json` entry sets `"takesQuery": true` implements
/// [`query::Guest`] too and calls [`query::export!`](crate::query::export)
/// beside [`export!`]:
///
/// ```ignore
/// pane_guest::export!(Echo);
/// pane_guest::query::export!(Echo);
/// ```
pub mod query {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "query-command-provider",
        pub_export_macro: true,
        default_bindings_module: "pane_guest::query",
    });

    pub use exports::pane::extension::query_command::Guest;
}

/// The applications installed on the system (`pane:extension/applications`),
/// which Pane finds and opens for the extension: [`applications::installed`] and
/// [`applications::open`].
pub mod applications {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "applications-user",
        default_bindings_module: "pane_guest::applications",
    });

    pub use pane::extension::applications::{Application, installed, open};
}

/// Clipboard history (`pane:extension/clipboard-history`), which Pane keeps
/// for the command's package once the user turned it on: plain text the
/// user copies while the package runs and the history is not paused, except
/// what the copying application marked as not to be kept or what came from
/// a program the user excluded. [`clipboard_history::set_capture`] turns it
/// on, off or pauses it; [`clipboard_history::entries`] lists what is kept.
pub mod clipboard_history {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "clipboard-history-user",
        default_bindings_module: "pane_guest::clipboard_history",
    });

    pub use pane::extension::clipboard_history::{
        Capture, Entry, HistoryStatus, clear, copy, entries, set_capture, set_excluded, status,
    };
}

/// Native helpers (`pane:extension/helpers`): prebuilt programs the
/// command's own package ships, one per system, which Pane runs for it with
/// [`helpers::run`]. Declare them under `helpers` in `pane.json`. Dropping
/// the future of a run before it resolves (for example when a timer wins a
/// race with it) cancels it: Pane ends the helper's process.
pub mod helpers {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "helpers-user",
        default_bindings_module: "pane_guest::helpers",
    });

    pub use pane::extension::helpers::{HelperError, HelperErrorKind, run};

    impl HelperErrorKind {
        /// The kind's WIT name, such as `not-found`.
        pub fn name(&self) -> &'static str {
            match self {
                HelperErrorKind::NotFound => "not-found",
                HelperErrorKind::Unavailable => "unavailable",
                HelperErrorKind::Failed => "failed",
                HelperErrorKind::Refused => "refused",
            }
        }
    }
}

/// The files of the folder the user granted the command's package
/// (`pane:extension/files`), which Pane lists for it under its scan limits
/// ([`files::limits`]): [`files::list_folder`] answers at once, with the
/// listing Pane keeps for this visit of root search, or that it is still
/// listing (Pane asks the command again when it is done), or that no folder
/// is granted. The package's `pane.json` sets `"folderAccess": true`; the
/// user chooses the folder in Pane's own row, and the extension never sees
/// its path. A command answers `open-file` results
/// ([`root::RootAction::OpenFile`]) with the files' ids.
pub mod files {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "files-user",
        default_bindings_module: "pane_guest::files",
    });

    pub use pane::extension::files::{
        FolderListing, FolderState, FoundFile, ScanLimits, limits, list_folder,
    };
}

/// Root results a command supplies ahead of the query
/// (`pane:extension/indexed-results`), such as the installed applications,
/// which root search matches by title like commands. A command whose
/// `pane.json` entry sets `"indexedResults": true` implements
/// [`indexed::Guest`] too and calls [`indexed::export!`](crate::indexed::export)
/// beside [`export!`]:
///
/// ```ignore
/// pane_guest::export!(Applications);
/// pane_guest::indexed::export!(Applications);
/// ```
pub mod indexed {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "indexed-results-provider",
        pub_export_macro: true,
        default_bindings_module: "pane_guest::indexed",
    });

    pub use exports::pane::extension::indexed_results::{Guest, IndexedAction, IndexedResult};
}

/// A command that searches as the user types into its own search field
/// (`pane:extension/command-search`), such as one searching an online
/// service. Pane asks it only once the user has opened it, never while they
/// type in root search. A command whose `pane.json` entry sets
/// `"search": true` implements [`search::Guest`] too and calls
/// [`search::export!`](crate::search::export) beside [`export!`]:
///
/// ```ignore
/// pane_guest::export!(Packages);
/// pane_guest::search::export!(Packages);
/// ```
pub mod search {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "command-search-provider",
        pub_export_macro: true,
        default_bindings_module: "pane_guest::search",
    });

    pub use exports::pane::extension::command_search::{Guest, SearchResult};
}

/// A command's scheduled task (`pane:extension/scheduled-task`), which Pane
/// runs in the background every so often once the user turns the command's
/// schedule on in Manage extensions. A command whose `pane.json` entry
/// declares a `schedule` (`"schedule": { "everyMinutes": 15 }`) implements
/// [`scheduled::Guest`] too and calls
/// [`scheduled::export!`](crate::scheduled::export) beside [`export!`]:
///
/// ```ignore
/// pane_guest::export!(Ticks);
/// pane_guest::scheduled::export!(Ticks);
/// ```
///
/// Each run starts in an instance of its own, so keep what the next run
/// needs in extension data, not in memory.
pub mod scheduled {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "scheduled-task-provider",
        pub_export_macro: true,
        default_bindings_module: "pane_guest::scheduled",
    });

    pub use exports::pane::extension::scheduled_task::Guest;
}

/// A command's continuing service (`pane:extension/service`), which Pane
/// keeps running in the background once the user starts it in Manage
/// extensions, and starts again with its package's code. A command whose
/// `pane.json` entry sets `"service": true` implements [`service::Guest`]
/// too and calls [`service::export!`](crate::service::export) beside
/// [`export!`]:
///
/// ```ignore
/// pane_guest::export!(Heartbeat);
/// pane_guest::service::export!(Heartbeat);
/// ```
///
/// The service runs in an instance of its own, awaiting what it watches in
/// a loop, and shows how it is doing with [`service::set_status`]. Await
/// between pieces of work: every extension's calls are served on one
/// thread.
pub mod service {
    #[doc(hidden)]
    pub mod provider {
        wit_bindgen::generate!({
            path: "../../wit",
            world: "service-provider",
            pub_export_macro: true,
            default_bindings_module: "pane_guest::service::provider",
        });
    }

    #[doc(hidden)]
    pub mod status {
        wit_bindgen::generate!({
            path: "../../wit",
            world: "service-status-user",
            default_bindings_module: "pane_guest::service::status",
        });
    }

    #[doc(inline)]
    pub use provider::export;
    pub use provider::exports::pane::extension::service::Guest;
    /// Shows `text` as the running service's status in Manage extensions;
    /// refused anywhere but in the service's own run.
    pub use status::pane::extension::service_status::set_status;
}

pub mod http;

/// The custom view type of a command that has none: `type CustomView =
/// NoCustomView;` in its `Guest` implementation, with an `open_view` that
/// returns `Err`. It has no values, so no view of it can be opened.
pub enum NoCustomView {}

impl GuestCustomView for NoCustomView {
    async fn render(&self) -> Frame {
        match *self {}
    }

    async fn handle_event(&self, _event: ViewEvent) -> Result<(), alloc::string::String> {
        match *self {}
    }
}

#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

/// Byte comparison, normally supplied by libc. The compiler emits calls to it
/// for slice and string comparisons (`==` on `str`, `starts_with`, ...).
///
/// # Safety
/// `a` and `b` must be valid for reads of `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(a: *const c_void, b: *const c_void, n: usize) -> i32 {
    let (a, b) = (a.cast::<u8>(), b.cast::<u8>());
    for i in 0..n {
        // Byte by byte through volatile reads, so that the compiler cannot
        // turn this loop back into a call to memcmp.
        let (x, y) = unsafe { (a.add(i).read_volatile(), b.add(i).read_volatile()) };
        if x != y {
            return i32::from(x) - i32::from(y);
        }
    }
    0
}

/// Equality-only form of [`memcmp`], which the compiler may call instead.
///
/// # Safety
/// `a` and `b` must be valid for reads of `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bcmp(a: *const c_void, b: *const c_void, n: usize) -> i32 {
    unsafe { memcmp(a, b, n) }
}

/// Canonical-ABI allocation entry point, normally supplied by `std`.
///
/// # Safety
/// Called only by the component runtime with valid allocation metadata.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cabi_realloc(
    old: *mut u8,
    old_size: usize,
    align: usize,
    new_size: usize,
) -> *mut u8 {
    use alloc::alloc::{Layout, alloc, realloc};
    if new_size == 0 {
        return align as *mut u8;
    }
    let result = if old_size == 0 {
        unsafe { alloc(Layout::from_size_align_unchecked(new_size, align)) }
    } else {
        unsafe {
            realloc(
                old,
                Layout::from_size_align_unchecked(old_size, align),
                new_size,
            )
        }
    };
    if result.is_null() {
        core::arch::wasm32::unreachable();
    }
    result
}
