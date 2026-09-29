//! Pane's core: the launcher model, extension packages and the extension
//! runtime the launcher drives.

pub mod applications;
mod atomic;
pub mod changes;
pub mod clock;
mod dependencies;
pub mod develop;
mod extension_data;
pub mod files;
mod generation;
mod helpers;
pub mod hotkeys;
mod http;
mod launcher;
mod links;
pub mod npm;
mod operations;
mod packages;
mod platform;
mod runtime;
mod search;

pub use helpers::runner::{MAX_HELPER_INPUT, MAX_HELPER_OUTPUT};
#[doc(hidden)]
pub use http::HttpLimits;
pub use launcher::{
    BackgroundChanges, BuildFailure, CommandRegistration, CustomViewSnapshot, Development,
    FormField, FormView, Launcher, LauncherView, Question, RESTART_DELAY, Row, ScheduledTask,
    Screen, ServiceOutcome, ServiceState, Status, TaskOutcome, Unavailable,
};
pub use links::LinkOpener;
pub use operations::{MAX_CALL_DEPTH, MAX_OPERATION_JSON};
pub use packages::{
    EXTENSION_API, InstalledPackage, MANIFEST_FILE, MANIFEST_VERSION, Manifest, ManifestCommand,
    ManifestHelper, ManifestOperation, PackageError, PackageIdentity, RetainedData, SavedData,
    Schedule,
};
pub use pane_target::{Arch, Target};
pub use platform::Platform;
#[cfg(debug_assertions)]
#[doc(hidden)]
pub use runtime::Fault;
pub use runtime::{
    CallError, Choice, CustomViewInfo, CustomViewRole, Field, FieldKind, FieldValue, Form,
    FormError, Frame, Item, Key, MAX_FRAME_SHAPES, MAX_FRAME_SIZE, MAX_TEXT_CHARS, Point, Rgb,
    Runtime, RuntimeStatus, Shape, View, ViewEvent, ViewId,
};
