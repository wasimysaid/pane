//! Extension packages: the `pane.json` manifest, source-derived package
//! identity, and Pane's managed copies of installed packages.
//!
//! A local package is a folder holding `pane.json` and the components it
//! names. Installing copies exactly those files into Pane's managed location,
//! so the user's folder is never written and the installed copy keeps working
//! if the folder changes or disappears; of a native helper (see `helpers`),
//! only its file for this system is copied. Installed packages are recorded in
//! `installed.json`, with whether the user disabled each; reading them back
//! needs only the manifests, never the guests.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component as PathPart, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::atomic::{Readers, write_atomically};
use crate::helpers::runner;
use crate::launcher::CommandRegistration;
use crate::npm::{Fetched, NpmOrigin, NpmPackage, NpmSpec};
use crate::platform::{self, Platform};
use crate::runtime::{CallError, Exports};
use pane_target::Target;

/// The manifest file at the root of every package.
pub const MANIFEST_FILE: &str = "pane.json";

/// The manifest format version this Pane reads.
pub const MANIFEST_VERSION: u64 = 1;

/// The extension API this Pane provides, as (major, minor): the
/// `pane:extension` WIT package version.
pub const EXTENSION_API: (u64, u64) = (0, 1);

const REGISTRY_FILE: &str = "installed.json";
const REGISTRY_VERSION: u64 = 1;
const PACKAGES_DIR: &str = "packages";

/// The identity of an installed package, derived from its source and
/// independent of its display title. A local package is identified by its
/// folder's resolved absolute path, as the operating system reports it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PackageIdentity(Source);

/// A package's source, as `installed.json` records it: `"local": "<folder>"`
/// or `"npm": "<package name>"`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
enum Source {
    Local { local: String },
    Npm { npm: String },
}

impl PackageIdentity {
    /// Resolves the identity of the local package folder at `folder`.
    ///
    /// The path is made absolute and resolved by the operating system
    /// (`std::fs::canonicalize`): symbolic links and `.`/`..` are followed,
    /// and file systems that ignore case or Unicode normalization report the
    /// stored spelling. Pane applies no case folding or normalization of its
    /// own, so on a case-sensitive file system two spellings are two folders.
    pub fn local(folder: &Path) -> Result<PackageIdentity, PackageError> {
        let resolved = fs::canonicalize(folder)
            .map_err(|error| PackageError::NotAFolder(folder.to_path_buf(), error.to_string()))?;
        if !resolved.is_dir() {
            return Err(PackageError::NotAFolder(
                folder.to_path_buf(),
                "it is not a folder".into(),
            ));
        }
        let resolved = without_verbatim_prefix(resolved);
        let path = resolved
            .to_str()
            .ok_or_else(|| PackageError::NotUnicode(resolved.clone()))?;
        Ok(PackageIdentity(Source::Local {
            local: path.to_owned(),
        }))
    }

    /// A stable key for this identity, for ids and records rather than for
    /// people to read: `local:` followed by the folder's resolved path. The
    /// [`Display`](fmt::Display) form is the wording shown to users.
    pub fn key(&self) -> String {
        match &self.0 {
            Source::Local { local } => format!("local:{local}"),
            Source::Npm { npm } => format!("npm:{npm}"),
        }
    }

    /// The identity of the npm package `name` (checked by
    /// [`NpmSpec::parse`]), whatever its version.
    pub fn npm(name: &str) -> PackageIdentity {
        PackageIdentity(Source::Npm {
            npm: name.to_owned(),
        })
    }

    /// The package name of an npm package.
    pub fn npm_name(&self) -> Option<&str> {
        match &self.0 {
            Source::Npm { npm } => Some(npm),
            Source::Local { .. } => None,
        }
    }

    /// The source folder of a local package.
    pub fn local_folder(&self) -> Option<&Path> {
        match &self.0 {
            Source::Local { local } => Some(Path::new(local)),
            Source::Npm { .. } => None,
        }
    }

    /// The identity of the package that the package with this identity
    /// names with the dependency source `source` (`local:` and a `/`-separated
    /// path, as the manifest checked). The path is relative to this package's
    /// source folder as Pane resolved it (a package installed through a
    /// symbolic link resolves against the folder the link points to), or
    /// absolute. An existing folder is resolved as an installed folder is.
    /// For one that does not exist (yet, or any more), `.` and `..` are
    /// removed from the spelling and its deepest existing parent is resolved
    /// by the operating system, so that it matches the identity the folder
    /// gets once it exists, unless the folder itself becomes a link, which
    /// [`PackageIdentity::resolved_again`] covers when calls are matched.
    /// Fails, with the path, only for a path that cannot be an identity (not
    /// absolute or not Unicode).
    ///
    /// An `npm:` source is the npm package it names, whatever its version.
    /// A package from npm cannot name a `local:` folder: its folder is on its
    /// author's computer, not the user's.
    pub(crate) fn dependency(&self, source: &str) -> Result<PackageIdentity, PathBuf> {
        let path = match SourceSpec::parse(source) {
            Ok(SourceSpec::Npm(spec)) => return Ok(PackageIdentity::npm(&spec.name)),
            Ok(SourceSpec::Local(path)) => path,
            Err(_) => return Err(PathBuf::from(source)),
        };
        let folder = match &self.0 {
            Source::Local { local } => Path::new(local).join(&path),
            Source::Npm { .. } => return Err(PathBuf::from(path)),
        };
        if let Ok(identity) = PackageIdentity::local(&folder) {
            return Ok(identity);
        }
        let mut spelled = PathBuf::new();
        for part in folder.components() {
            match part {
                PathPart::CurDir => {}
                PathPart::ParentDir => {
                    spelled.pop();
                }
                other => spelled.push(other),
            }
        }
        let mut missing = Vec::new();
        let mut existing = spelled.clone();
        let resolved = loop {
            if let Ok(resolved) = fs::canonicalize(&existing) {
                break missing
                    .iter()
                    .rev()
                    .fold(resolved, |path: PathBuf, part| path.join(part));
            }
            match (
                existing.file_name().map(ToOwned::to_owned),
                existing.parent(),
            ) {
                (Some(name), Some(parent)) => {
                    missing.push(name);
                    existing = parent.to_path_buf();
                }
                _ => break spelled.clone(),
            }
        };
        let resolved = without_verbatim_prefix(resolved);
        match resolved.to_str() {
            Some(text) if resolved.is_absolute() => Ok(PackageIdentity(Source::Local {
                local: text.to_owned(),
            })),
            _ => Err(resolved),
        }
    }

    /// This local identity resolved by the operating system now, if its
    /// folder exists and resolves to another spelling: a dependency recorded
    /// before its folder existed, which became a symbolic link or was
    /// created with another spelling on a file system that ignores case.
    pub(crate) fn resolved_again(&self) -> Option<PackageIdentity> {
        let resolved = PackageIdentity::local(self.local_folder()?).ok()?;
        (resolved != *self).then_some(resolved)
    }
}

/// The installed package with `identity` among `packages`.
pub(crate) fn installed_as<'a>(
    packages: &'a [InstalledPackage],
    identity: &PackageIdentity,
) -> Option<&'a InstalledPackage> {
    packages
        .iter()
        .find(|package| package.identity == *identity)
}

impl fmt::Display for PackageIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Source::Local { local } => write!(f, "local folder {local}"),
            Source::Npm { npm } => write!(f, "npm package {npm}"),
        }
    }
}

/// The name people know a package folder by: its last component, or the
/// whole path when it has none.
pub(crate) fn folder_name(folder: &Path) -> String {
    folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.display().to_string())
}

/// `path` resolved as a package identity's folder is: canonical, in the
/// ordinary spelling on Windows.
pub(crate) fn canonical(path: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(path).map(without_verbatim_prefix)
}

/// Windows' canonical paths carry a `\\?\` prefix; the identity uses the
/// ordinary spelling (`C:\…`, `\\server\share\…`) that users recognise.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        PathBuf::from(rest)
    } else {
        path
    }
}

/// A package's manifest, `pane.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub title: String,
    pub version: Option<String>,
    /// The extension API the package needs, such as `0.1`.
    pub api_version: String,
    /// The operating systems the package supports; `None` when it does not
    /// say, which means every system Pane runs on, and empty for none. A
    /// package that does not support this system is explained instead of
    /// installed, and an installed copy of one lists its commands as
    /// unavailable.
    pub platforms: Option<Vec<Platform>>,
    pub commands: Vec<ManifestCommand>,
    /// The operations the package publishes for other extensions to call.
    /// Only these are callable: a command is not an operation.
    pub operations: Vec<ManifestOperation>,
    /// The native helpers the package ships, which its commands run through
    /// Pane (`pane:extension/helpers`).
    pub helpers: Vec<ManifestHelper>,
    /// The other packages whose operations this one calls, required or
    /// optional.
    pub dependencies: Vec<ManifestDependency>,
    /// The package asks for access to one folder the user chooses
    /// (`"folderAccess": true`): Pane offers its own "Choose folder" row
    /// in the package's commands, and lists only that folder for it
    /// (`pane:extension/files`).
    pub folder_access: bool,
}

/// A native helper a package ships: a prebuilt program per target (operating
/// system and processor), which its commands run by name through Pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestHelper {
    pub id: String,
    /// The helper's file for each target it is built for, such as
    /// `linux-x86_64`, relative to the package folder.
    pub targets: BTreeMap<Target, PathBuf>,
}

impl ManifestHelper {
    /// The helper's file for this system, if the package ships one.
    pub fn for_this_system(&self) -> Option<&Path> {
        self.targets.get(&Target::current()?).map(PathBuf::as_path)
    }
}

/// Another package whose operations a package calls, as its `pane.json`
/// declares it under `dependencies`. Installing the package installs its
/// missing required dependencies with it; an optional one is used only when
/// the user installed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestDependency {
    /// The name the package's code calls it by in place of its package
    /// identity: unique in the package; lowercase letters, digits and `-`.
    pub id: String,
    /// Where it is installed from, as written: `local:` and a folder path
    /// separated by `/`, relative to the declaring package's folder or
    /// absolute (`/…`), or `npm:` and a package name, optionally with an
    /// exact version (`npm:@scope/name@1.2.3`). Other sources are not
    /// supported yet.
    pub source: String,
    /// Whether the package needs it (the default) or only uses it when it is
    /// installed (`"optional": true`).
    pub required: bool,
    /// The operations the package calls, each at the version it calls: the
    /// dependency is compatible when it publishes all of them.
    pub operations: Vec<RequiredOperation>,
    /// The operating systems on which the package needs it; `None` for
    /// every system. Elsewhere it is neither installed nor checked.
    pub platforms: Option<Vec<Platform>>,
}

impl ManifestDependency {
    /// Whether the declaring package needs this dependency on this system.
    pub fn needed_here(&self) -> bool {
        self.only_on().is_none()
    }

    /// "only on Windows and Linux" when the package does not need it on
    /// this system; `None` when it does.
    pub(crate) fn only_on(&self) -> Option<String> {
        platform::unavailable(self.platforms.as_deref(), "it")?;
        let platforms = self.platforms.as_deref().unwrap_or_default();
        Some(match platforms {
            [] => "on no system".to_owned(),
            platforms => format!("only on {}", platform::names(platforms)),
        })
    }

    /// Whether the package declares that it calls `operation` at `version`
    /// here.
    pub(crate) fn calls(&self, operation: &str, version: u32) -> bool {
        self.operations
            .iter()
            .any(|declared| declared.id == operation && declared.version == version)
    }
}

/// An operation a package calls in one of its dependencies, at the version
/// it calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequiredOperation {
    pub id: String,
    pub version: u32,
}

/// An operation a package publishes: other extensions call it through
/// Pane by the package's source, the operation's id and its version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestOperation {
    pub id: String,
    /// The version of the operation's input and result: a caller names the
    /// version it was written for, and any other is refused. A change that
    /// breaks callers publishes a new version.
    pub version: u32,
    /// The component serving it, relative to the package folder; often a
    /// command's component too.
    pub component: PathBuf,
    /// The operating systems it works on; `None` for every system the
    /// package supports. Elsewhere a call to it is unavailable.
    pub platforms: Option<Vec<Platform>>,
}

/// A command a package contributes to root search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestCommand {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    /// The command's component, relative to the package folder.
    pub component: PathBuf,
    /// The operating systems the command supports; `None` for every system
    /// the package supports. Elsewhere it is listed as unavailable.
    pub platforms: Option<Vec<Platform>>,
    /// Whether the command computes root results from root search's query
    /// (`"rootResults": true`), such as a calculator's answer: its component
    /// then also exports `pane:extension/root-results`.
    pub root_results: bool,
    /// Whether the command supplies root results ahead of the query
    /// (`"indexedResults": true`), such as the installed applications: its
    /// component then also exports `pane:extension/indexed-results`.
    pub indexed_results: bool,
    /// Whether the command takes a query (`"takesQuery": true`): text typed
    /// into root search that Pane sends it when the user invokes it through
    /// its alias or as a fallback. Its component then also exports
    /// `pane:extension/query-command`.
    pub takes_query: bool,
    /// Whether the command searches as the user types into its own search
    /// field once it is open (`"search": true`), such as a command searching
    /// an online service; root search never asks it. Its component then
    /// also exports `pane:extension/command-search`.
    pub search: bool,
    /// How often the command's scheduled task runs once the user turns its
    /// schedule on (`"schedule": { "everyMinutes": 15 }`); `None` for a
    /// command without one. Its component then also exports
    /// `pane:extension/scheduled-task`.
    pub schedule: Option<Schedule>,
}

/// The one kind of schedule a command can declare: every so many minutes,
/// from one minute to a day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    every_minutes: u32,
}

impl Schedule {
    /// The shortest schedule: every minute.
    pub const MIN_MINUTES: u32 = 1;
    /// The longest schedule: once a day.
    pub const MAX_MINUTES: u32 = 24 * 60;

    /// How long after a run began the next one is due.
    pub fn every(&self) -> std::time::Duration {
        std::time::Duration::from_secs(u64::from(self.every_minutes) * 60)
    }
}

impl fmt::Display for Schedule {
    /// "every minute", "every 15 minutes", "every hour", "every 2 hours".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.every_minutes {
            1 => f.write_str("every minute"),
            60 => f.write_str("every hour"),
            minutes if minutes % 60 == 0 => write!(f, "every {} hours", minutes / 60),
            minutes => write!(f, "every {minutes} minutes"),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScheduleJson {
    every_minutes: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestJson {
    title: String,
    #[serde(default)]
    version: Option<String>,
    api_version: String,
    #[serde(default)]
    platforms: Option<Vec<String>>,
    #[serde(default)]
    commands: Vec<CommandJson>,
    #[serde(default)]
    operations: Vec<OperationJson>,
    #[serde(default)]
    helpers: Vec<HelperJson>,
    #[serde(default)]
    dependencies: Vec<DependencyJson>,
    #[serde(default)]
    folder_access: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HelperJson {
    id: String,
    targets: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DependencyJson {
    id: String,
    source: String,
    #[serde(default)]
    optional: bool,
    operations: Vec<RequiredOperationJson>,
    #[serde(default)]
    platforms: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequiredOperationJson {
    id: String,
    version: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationJson {
    id: String,
    version: u32,
    component: String,
    #[serde(default)]
    platforms: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommandJson {
    id: String,
    title: String,
    #[serde(default)]
    subtitle: Option<String>,
    component: String,
    #[serde(default)]
    platforms: Option<Vec<String>>,
    #[serde(default)]
    root_results: bool,
    #[serde(default)]
    indexed_results: bool,
    #[serde(default)]
    takes_query: bool,
    #[serde(default)]
    search: bool,
    #[serde(default)]
    schedule: Option<ScheduleJson>,
}

impl Manifest {
    /// Reads and validates `pane.json` in `folder`, including that the
    /// package supports this operating system and that every component it
    /// names is present. Runs no guest code.
    pub fn read(folder: &Path) -> Result<Manifest, PackageError> {
        Manifest::read_text(folder).map(|(manifest, _)| manifest)
    }

    /// Like [`Manifest::read`], also returning the text that was validated.
    fn read_text(folder: &Path) -> Result<(Manifest, String), PackageError> {
        let (manifest, text) = Manifest::read_parsed(folder)?;
        if let Some(reason) = platform::unavailable(manifest.platforms.as_deref(), "this package") {
            return Err(PackageError::UnsupportedPlatform(reason));
        }
        manifest.check_components(folder)?;
        Ok((manifest, text))
    }

    /// Reads a managed copy: like [`Manifest::read`], but a copy for other
    /// systems is read, so that its commands can be listed as unavailable.
    fn read_installed(folder: &Path) -> Result<Manifest, PackageError> {
        let (manifest, _) = Manifest::read_parsed(folder)?;
        manifest.check_components(folder)?;
        Ok(manifest)
    }

    /// Reads and parses `pane.json` in `folder`, without checking that its
    /// components exist (a development build is about to make them).
    pub(crate) fn read_parsed(folder: &Path) -> Result<(Manifest, String), PackageError> {
        let path = folder.join(MANIFEST_FILE);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(PackageError::NoManifest(folder.to_path_buf()));
            }
            Err(error) => return Err(PackageError::InvalidManifest(error.to_string())),
        };
        let manifest = Manifest::parse(&text)?;
        Ok((manifest, text))
    }

    /// Checks that every component the manifest names is in `folder`, and
    /// that each helper's file for this system, where the package ships
    /// one, is a regular file inside the package and a program for this
    /// system ([`runner::check_file`]).
    fn check_components(&self, folder: &Path) -> Result<(), PackageError> {
        for (name, component) in self.components() {
            if !folder.join(component).is_file() {
                return Err(PackageError::MissingComponent {
                    command: name,
                    component: component.to_path_buf(),
                });
            }
        }
        let Some(target) = Target::current() else {
            return Ok(());
        };
        for helper in &self.helpers {
            let Some(file) = helper.for_this_system() else {
                continue;
            };
            if let Err(reason) = runner::check_file(folder, file, target) {
                return Err(PackageError::Helper {
                    helper: helper.id.clone(),
                    target,
                    reason,
                });
            }
        }
        Ok(())
    }

    /// Every component the manifest names, relative to the package folder,
    /// with what it serves as people know it: a command's title, or
    /// "operation `<id>`". A component serving several appears once per use.
    pub(crate) fn components(&self) -> impl Iterator<Item = (String, &Path)> {
        let commands = self
            .commands
            .iter()
            .map(|command| (command.title.clone(), command.component.as_path()));
        let operations = self.operations.iter().map(|operation| {
            (
                format!("operation `{}`", operation.id),
                operation.component.as_path(),
            )
        });
        commands.chain(operations)
    }

    /// What `component` exports besides `command`, as the manifest says.
    pub(crate) fn exports_of(&self, component: &Path) -> Exports {
        let commands = || {
            self.commands
                .iter()
                .filter(|command| command.component == component)
        };
        Exports {
            root_results: commands().any(|command| command.root_results),
            indexed_results: commands().any(|command| command.indexed_results),
            query_command: commands().any(|command| command.takes_query),
            search: commands().any(|command| command.search),
            scheduled_task: commands().any(|command| command.schedule.is_some()),
            operations: self
                .operations
                .iter()
                .any(|operation| operation.component == component),
        }
    }

    fn parse(text: &str) -> Result<Manifest, PackageError> {
        let invalid = |message: String| PackageError::InvalidManifest(message);
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|error| invalid(error.to_string()))?;
        // The format version is checked first: a newer manifest may not match
        // this version's fields at all.
        match value.get("manifestVersion").map(serde_json::Value::as_u64) {
            Some(Some(MANIFEST_VERSION)) => {}
            Some(Some(found)) if found > MANIFEST_VERSION => {
                return Err(PackageError::NewerManifest(found));
            }
            Some(_) => return Err(invalid("manifestVersion must be 1".into())),
            None => return Err(invalid("missing field `manifestVersion`".into())),
        }
        let json: ManifestJson =
            serde_json::from_value(value).map_err(|error| invalid(error.to_string()))?;
        if !api_compatible(&json.api_version)? {
            return Err(PackageError::IncompatibleApi(json.api_version));
        }
        if json.title.trim().is_empty() {
            return Err(invalid("`title` is empty".into()));
        }
        let platforms = parse_platforms(json.platforms, "`platforms`")?;
        if json.commands.is_empty() && json.operations.is_empty() {
            return Err(invalid("`commands` is empty".into()));
        }
        let mut commands = Vec::new();
        for command in json.commands {
            if command.id.is_empty() || command.title.trim().is_empty() {
                return Err(invalid("every command needs an `id` and a `title`".into()));
            }
            // Pane's records name a command `<package identity>#<id>`; an id
            // without `#` keeps the package's part of that unambiguous.
            if command.id.contains('#') {
                return Err(invalid(format!(
                    "command id `{}` contains `#`, which command ids cannot",
                    command.id
                )));
            }
            if commands
                .iter()
                .any(|seen: &ManifestCommand| seen.id == command.id)
            {
                return Err(invalid(format!("command id `{}` is repeated", command.id)));
            }
            // Root search never asks a command that searches inside itself:
            // results it computed for root search would never be shown.
            if command.search && command.root_results {
                return Err(invalid(format!(
                    "command `{}` sets both `search` and `rootResults`: a command that \
                     searches inside itself is never asked by root search",
                    command.id
                )));
            }
            let component = inside_package(&command.component, "component")?;
            let platforms = parse_platforms(
                command.platforms,
                &format!("`platforms` of command `{}`", command.id),
            )?;
            let schedule = match command.schedule {
                Some(ScheduleJson { every_minutes })
                    if (Schedule::MIN_MINUTES..=Schedule::MAX_MINUTES).contains(&every_minutes) =>
                {
                    Some(Schedule { every_minutes })
                }
                Some(ScheduleJson { every_minutes }) => {
                    return Err(invalid(format!(
                        "the schedule of command `{}` runs every {every_minutes} minutes; it \
                         must be from {} to {} (a day)",
                        command.id,
                        Schedule::MIN_MINUTES,
                        Schedule::MAX_MINUTES
                    )));
                }
                None => None,
            };
            commands.push(ManifestCommand {
                id: command.id,
                title: command.title,
                subtitle: command.subtitle,
                component,
                platforms,
                root_results: command.root_results,
                indexed_results: command.indexed_results,
                takes_query: command.takes_query,
                search: command.search,
                schedule,
            });
        }
        let mut operations: Vec<ManifestOperation> = Vec::new();
        for operation in json.operations {
            if operation.id.is_empty() {
                return Err(invalid("every operation needs an `id`".into()));
            }
            if operations.iter().any(|seen| seen.id == operation.id) {
                return Err(invalid(format!(
                    "operation id `{}` is repeated",
                    operation.id
                )));
            }
            if operation.version == 0 {
                return Err(invalid(format!(
                    "operation `{}` has version 0; versions start at 1",
                    operation.id
                )));
            }
            let platforms = parse_platforms(
                operation.platforms,
                &format!("`platforms` of operation `{}`", operation.id),
            )?;
            operations.push(ManifestOperation {
                platforms,
                component: inside_package(&operation.component, "component")?,
                id: operation.id,
                version: operation.version,
            });
        }
        let mut helpers: Vec<ManifestHelper> = Vec::new();
        for helper in json.helpers {
            if let Some(problem) = runner::id_problem(&helper.id) {
                return Err(invalid(problem));
            }
            if helpers.iter().any(|seen| seen.id == helper.id) {
                return Err(invalid(format!("helper id `{}` is repeated", helper.id)));
            }
            if helper.targets.is_empty() {
                return Err(invalid(format!(
                    "helper `{}` has no `targets`; name its file for each system it is \
                     built for, such as \"linux-x86_64\"",
                    helper.id
                )));
            }
            let mut targets = BTreeMap::new();
            for (id, file) in helper.targets {
                let Some(target) = Target::parse(&id) else {
                    return Err(invalid(format!(
                        "unknown target `{}` of helper `{}`; use windows, macos or linux, \
                         a dash, and x86_64 or aarch64, such as \"linux-x86_64\"",
                        id.escape_debug(),
                        helper.id
                    )));
                };
                let file = inside_package(&file, "helper file")?;
                if let Some(problem) = runner::name_problem(&file, target) {
                    return Err(invalid(format!("helper `{}`: {problem}", helper.id)));
                }
                targets.insert(target, file);
            }
            helpers.push(ManifestHelper {
                id: helper.id,
                targets,
            });
        }
        let dependencies = parse_dependencies(json.dependencies)?;
        Ok(Manifest {
            title: json.title,
            version: json.version,
            api_version: json.api_version,
            platforms,
            commands,
            operations,
            helpers,
            dependencies,
            folder_access: json.folder_access,
        })
    }

    /// The dependency the package's code calls `id`.
    pub fn dependency(&self, id: &str) -> Option<&ManifestDependency> {
        self.dependencies
            .iter()
            .find(|dependency| dependency.id == id)
    }
}

/// A package source as it is written: in a manifest's `dependencies`, or
/// by a caller naming a package by its identity. It is `local:` and a
/// folder path, or `npm:` and a package name with an optional exact
/// version. This is the one place such text is read; a Git source (Q10)
/// would be another kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceSpec {
    /// `local:<path>`: the path as written, not yet checked or resolved.
    Local(String),
    /// `npm:<name>[@<version>]`.
    Npm(NpmSpec),
}

/// Why text is not a [`SourceSpec`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceError {
    /// It starts with neither `local:` nor `npm:`.
    UnknownKind,
    /// It is `npm:` and not a package name with an optional exact version,
    /// for this reason.
    Npm(String),
}

impl SourceSpec {
    pub(crate) fn parse(text: &str) -> Result<SourceSpec, SourceError> {
        match text.split_once(':') {
            Some(("local", path)) => Ok(SourceSpec::Local(path.to_owned())),
            Some(("npm", spec)) => NpmSpec::parse(spec)
                .map(SourceSpec::Npm)
                .map_err(SourceError::Npm),
            _ => Err(SourceError::UnknownKind),
        }
    }
}

/// Checks that `source`, of dependency `id`, is either `npm:` and a package
/// name with an optional exact version (`npm:greeter@1.2.3`), or `local:` and
/// a folder path written the same way on every system: `/` between folders,
/// relative or absolute from `/`, never with `\`, a drive letter or a
/// `//server` share, which one system would read differently from another.
fn check_source(source: &str, id: &str) -> Result<(), PackageError> {
    let invalid = |reason: &str| {
        Err(PackageError::InvalidManifest(format!(
            "the source `{source}` of dependency `{id}` {reason}"
        )))
    };
    let path = match SourceSpec::parse(source) {
        Ok(SourceSpec::Npm(_)) => return Ok(()),
        Ok(SourceSpec::Local(path)) => path,
        Err(SourceError::Npm(why)) => return invalid(&format!("is not an npm package: {why}")),
        Err(SourceError::UnknownKind) => {
            return invalid(
                "must be `local:` followed by a folder path or `npm:` followed by a package \
                 name; other sources are not supported yet",
            );
        }
    };
    if path.is_empty() {
        return invalid("must be `local:` followed by a folder path");
    }
    let drive =
        path.len() >= 2 && path.as_bytes()[1] == b':' && path.as_bytes()[0].is_ascii_alphabetic();
    if path.contains('\\') || drive || path.starts_with("//") {
        return invalid(
            "must separate folders with `/`, without a drive letter, `\\` or a `//server` \
             share, so that every system reads it alike (such as `local:../greeter`)",
        );
    }
    Ok(())
}

fn parse_dependencies(json: Vec<DependencyJson>) -> Result<Vec<ManifestDependency>, PackageError> {
    let invalid = |message: String| PackageError::InvalidManifest(message);
    let mut dependencies: Vec<ManifestDependency> = Vec::new();
    for dependency in json {
        let id = dependency.id;
        let valid_id = !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !valid_id {
            return Err(invalid(format!(
                "dependency id `{id}` must be lowercase letters, digits and `-`"
            )));
        }
        if dependencies.iter().any(|seen| seen.id == id) {
            return Err(invalid(format!("dependency id `{id}` is repeated")));
        }
        check_source(&dependency.source, &id)?;
        let mut operations: Vec<RequiredOperation> = Vec::new();
        for operation in dependency.operations {
            if operation.id.is_empty() || operation.version == 0 {
                return Err(invalid(format!(
                    "every operation of dependency `{id}` needs an `id` and a `version` from 1"
                )));
            }
            if operations.iter().any(|seen| seen.id == operation.id) {
                return Err(invalid(format!(
                    "operation `{}` of dependency `{id}` is repeated",
                    operation.id
                )));
            }
            operations.push(RequiredOperation {
                id: operation.id,
                version: operation.version,
            });
        }
        if operations.is_empty() {
            return Err(invalid(format!(
                "dependency `{id}` lists no `operations`; name those the package calls"
            )));
        }
        let platforms = parse_platforms(
            dependency.platforms,
            &format!("`platforms` of dependency `{id}`"),
        )?;
        dependencies.push(ManifestDependency {
            id,
            source: dependency.source,
            required: !dependency.optional,
            operations,
            platforms,
        });
    }
    Ok(dependencies)
}

/// `file` (a `what`, such as a component) as a path, if it is a relative
/// path inside the package folder.
fn inside_package(file: &str, what: &str) -> Result<PathBuf, PackageError> {
    let path = PathBuf::from(file);
    let inside = path
        .components()
        .all(|part| matches!(part, PathPart::Normal(_)));
    if !inside || file.is_empty() {
        return Err(PackageError::InvalidManifest(format!(
            "{what} `{file}` must be a relative path inside the package folder"
        )));
    }
    Ok(path)
}

/// Reads a `platforms` list (named `field` in explanations): `None` when
/// absent, and possibly empty, meaning no operating system.
fn parse_platforms(
    ids: Option<Vec<String>>,
    field: &str,
) -> Result<Option<Vec<Platform>>, PackageError> {
    ids.map(|ids| {
        ids.iter()
            .map(|id| {
                Platform::from_id(id).ok_or_else(|| {
                    PackageError::InvalidManifest(format!(
                        "unknown platform `{id}` in {field}; use windows, macos or linux"
                    ))
                })
            })
            .collect()
    })
    .transpose()
}

/// Whether a package needing extension API `required` (`MAJOR.MINOR`, with
/// an optional `.PATCH`) runs on this Pane. Before 1.0 each minor version is
/// its own API; from 1.0 an older minor version of the same major is served.
fn api_compatible(required: &str) -> Result<bool, PackageError> {
    let parts: Vec<Option<u64>> = required.split('.').map(|part| part.parse().ok()).collect();
    let (major, minor) = match parts.as_slice() {
        [Some(major), Some(minor)] | [Some(major), Some(minor), Some(_)] => (*major, *minor),
        _ => {
            return Err(PackageError::InvalidManifest(format!(
                "apiVersion `{required}` is not a version such as `0.1`"
            )));
        }
    };
    let (host_major, host_minor) = EXTENSION_API;
    Ok(if major == 0 {
        host_major == 0 && minor == host_minor
    } else {
        major == host_major && minor <= host_minor
    })
}

/// Why a package cannot be previewed, installed or updated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageError {
    /// The source is not a readable folder.
    NotAFolder(PathBuf, String),
    /// The folder's path cannot be recorded because it is not valid Unicode.
    NotUnicode(PathBuf),
    /// The folder has no `pane.json`.
    NoManifest(PathBuf),
    /// `pane.json` is not a valid manifest.
    InvalidManifest(String),
    /// `pane.json` uses a newer manifest format than this Pane reads.
    NewerManifest(u64),
    /// The package needs an extension API this Pane does not provide.
    IncompatibleApi(String),
    /// The package does not support this operating system; the reason says
    /// which ones it supports.
    UnsupportedPlatform(String),
    /// A component the manifest names is not in the folder.
    MissingComponent { command: String, component: PathBuf },
    /// The package ships a helper for this system whose file is missing or
    /// is not a program for this system.
    Helper {
        helper: String,
        target: Target,
        reason: String,
    },
    /// A component is present but Pane cannot run it.
    Component { command: String, error: CallError },
    /// A package with this identity is already installed.
    AlreadyInstalled(PackageIdentity),
    /// No package with this identity is installed.
    NotInstalled(PackageIdentity),
    /// Pane's managed location could not be read or written.
    Storage(String),
    /// A required dependency cannot be installed or used, for these
    /// reasons.
    Dependencies(Vec<String>),
    /// A package from npm cannot be downloaded, unpacked or installed; the
    /// message says why.
    Npm(String),
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageError::NotAFolder(path, reason) => {
                write!(f, "Cannot open {}: {reason}", path.display())
            }
            PackageError::NotUnicode(path) => write!(
                f,
                "Cannot install from {}: Pane needs a folder path that is valid Unicode",
                path.display()
            ),
            PackageError::NoManifest(path) => write!(
                f,
                "Not an extension package: {} has no {MANIFEST_FILE}",
                path.display()
            ),
            PackageError::InvalidManifest(reason) => {
                write!(f, "Invalid {MANIFEST_FILE}: {reason}")
            }
            PackageError::NewerManifest(found) => write!(
                f,
                "Incompatible package: its {MANIFEST_FILE} uses manifest version {found}, but this Pane reads version {MANIFEST_VERSION}; a newer Pane is needed"
            ),
            PackageError::IncompatibleApi(required) => write!(
                f,
                "Incompatible package: it needs Pane extension API {required}, but this Pane provides {}.{}",
                EXTENSION_API.0, EXTENSION_API.1
            ),
            PackageError::UnsupportedPlatform(reason) => f.write_str(reason),
            PackageError::MissingComponent { command, component } => write!(
                f,
                "Not ready to run: the component {} of \"{command}\" is missing. This looks like a source-only package; build its component before installing",
                component.display()
            ),
            PackageError::Helper {
                helper,
                target,
                reason,
            } => write!(
                f,
                "Not ready to run: the package ships helper `{helper}` for {target}, but {reason}"
            ),
            PackageError::Component { command, error } => write!(f, "\"{command}\": {error}"),
            PackageError::AlreadyInstalled(identity) => write!(
                f,
                "Already installed from {identity}; use Update to replace the installed copy"
            ),
            PackageError::NotInstalled(identity) => {
                write!(f, "Nothing is installed from {identity}")
            }
            PackageError::Storage(reason) => {
                write!(f, "Could not update Pane's installed extensions: {reason}")
            }
            PackageError::Dependencies(problems) => {
                write!(f, "Nothing was installed: {}", problems.join("; "))
            }
            PackageError::Npm(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for PackageError {}

/// A package in a source folder, read and validated but not installed.
#[derive(Clone, Debug)]
pub(crate) struct SourcePackage {
    pub identity: PackageIdentity,
    pub folder: PathBuf,
    pub manifest: Manifest,
    /// The `pane.json` text `manifest` was validated from; the managed copy
    /// gets exactly this, even if the source changes meanwhile.
    manifest_text: String,
    /// Where a package from npm was downloaded from; `None` for a local
    /// folder.
    pub npm: Option<NpmOrigin>,
    /// For a package from npm, its download, removed from the downloads
    /// folder once the last copy of this package is dropped.
    _download: Option<std::sync::Arc<crate::npm::Download>>,
    /// Whether a component of it imports `wasi:http` (it can make web
    /// requests), as checking its components found; `false` until they are
    /// checked.
    pub network: bool,
}

impl SourcePackage {
    /// Reads the npm package that Pane downloaded and unpacked, as the
    /// package with its npm identity. Explains, rather than as for a folder,
    /// a tarball without `pane.json` (an ordinary npm package, which Pane
    /// does not run) and one without its built components.
    pub(crate) fn read_npm(fetched: Fetched) -> Result<SourcePackage, PackageError> {
        let Fetched { download, origin } = fetched;
        let folder = download.folder().to_path_buf();
        let name = origin.package.name.clone();
        let spec = format!("{name}@{}", origin.package.version);
        let (manifest, manifest_text) = match Manifest::read_text(&folder) {
            Ok(read) => read,
            Err(PackageError::NoManifest(_)) => {
                return Err(PackageError::Npm(format!(
                    "npm package {spec} is not a Pane extension: it has no {MANIFEST_FILE}. Pane \
                     installs npm packages published as Pane extensions (a {MANIFEST_FILE} and \
                     the WebAssembly components it names); it does not run other npm packages, \
                     which need Node.js and npm"
                )));
            }
            Err(PackageError::MissingComponent { command, component }) => {
                let scripts = match origin.scripts.as_slice() {
                    [] => String::new(),
                    scripts => format!(
                        " (its package.json has {}, which Pane never runs)",
                        crate::platform::join(
                            &scripts.iter().map(|s| format!("`{s}`")).collect::<Vec<_>>()
                        )
                    ),
                };
                return Err(PackageError::Npm(format!(
                    "npm package {spec} was published without the built component {} of \
                     \"{command}\": its author must build it and include it in the package \
                     before publishing. Pane does not build npm packages or run their install \
                     scripts{scripts}",
                    component.display()
                )));
            }
            Err(error) => return Err(error),
        };
        Ok(SourcePackage {
            identity: PackageIdentity::npm(&name),
            folder,
            manifest,
            manifest_text,
            npm: Some(origin),
            _download: Some(std::sync::Arc::new(download)),
            network: false,
        })
    }

    /// Reads the package staged in `folder`, such as a development build,
    /// as the package with the source `identity`.
    pub fn read_staged(
        folder: &Path,
        identity: PackageIdentity,
    ) -> Result<SourcePackage, PackageError> {
        let (manifest, manifest_text) = Manifest::read_text(folder)?;
        Ok(SourcePackage {
            identity,
            folder: folder.to_path_buf(),
            manifest,
            manifest_text,
            npm: None,
            _download: None,
            network: false,
        })
    }

    pub fn read(folder: &Path) -> Result<SourcePackage, PackageError> {
        let identity = PackageIdentity::local(folder)?;
        let folder = identity
            .local_folder()
            .expect("a local identity has a folder")
            .to_path_buf();
        let (manifest, manifest_text) = Manifest::read_text(&folder)?;
        Ok(SourcePackage {
            identity,
            folder,
            manifest,
            manifest_text,
            npm: None,
            _download: None,
            network: false,
        })
    }

    /// The `pane.json` text the manifest was read from.
    pub(crate) fn manifest_text(&self) -> &str {
        &self.manifest_text
    }
}

/// An installed package, as read from its managed copy.
#[derive(Clone, Debug)]
pub struct InstalledPackage {
    pub identity: PackageIdentity,
    /// The manifest of the managed copy, or why it could not be read.
    pub manifest: Result<Manifest, PackageError>,
    /// Where Pane keeps this package's files.
    pub location: PathBuf,
    /// Whether the user has left the package enabled. A disabled package
    /// contributes no commands and runs nothing, but keeps its settings.
    pub enabled: bool,
    /// For a package from npm, the npm version installed and whether it is
    /// pinned to it.
    pub npm: Option<NpmPackage>,
    /// Whether a component of it imports `wasi:http`, so its code can make
    /// web requests, as found when it was installed, updated or reloaded.
    pub uses_network: bool,
    /// The identity each dependency the manifest declares was resolved to
    /// when the package was installed, by dependency id.
    dependencies: Vec<(String, PackageIdentity)>,
}

impl InstalledPackage {
    /// The package whose managed copy is at `location`, with the identities
    /// its dependencies were resolved to as `recorded`; one not recorded
    /// (installed before Pane recorded them) is resolved now.
    fn load(
        identity: PackageIdentity,
        location: PathBuf,
        enabled: bool,
        uses_network: bool,
        recorded: &[ResolvedJson],
        npm: Option<&NpmRecordJson>,
    ) -> InstalledPackage {
        let manifest = Manifest::read_installed(&location);
        let dependencies = match &manifest {
            Ok(manifest) => manifest
                .dependencies
                .iter()
                .filter_map(|dependency| {
                    let resolved = match recorded.iter().find(|r| r.id == dependency.id) {
                        Some(record) => PackageIdentity(record.source.clone()),
                        None => identity.dependency(&dependency.source).ok()?,
                    };
                    Some((dependency.id.clone(), resolved))
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        let npm = npm.and_then(|npm| npm.package(&identity));
        InstalledPackage {
            manifest,
            identity,
            location,
            enabled,
            npm,
            uses_network,
            dependencies,
        }
    }

    /// The identity of the package this one's code calls by the dependency
    /// id `id`, as resolved when it was installed; `None` if its manifest
    /// declares no such dependency, or not one from a local folder.
    pub fn dependency_identity(&self, id: &str) -> Option<&PackageIdentity> {
        self.dependencies
            .iter()
            .find(|(declared, _)| declared == id)
            .map(|(_, identity)| identity)
    }

    /// The package's display title.
    pub fn title(&self) -> String {
        match &self.manifest {
            Ok(manifest) => manifest.title.clone(),
            Err(_) => match (self.identity.local_folder(), self.identity.npm_name()) {
                (Some(folder), _) => folder_name(folder),
                (None, Some(name)) => name.to_owned(),
                (None, None) => self.identity.to_string(),
            },
        }
    }

    pub fn version(&self) -> Option<String> {
        self.manifest.as_ref().ok()?.version.clone()
    }

    /// The commands this package offers in root search.
    pub fn commands(&self) -> Vec<CommandRegistration> {
        self.available_commands()
            .into_iter()
            .map(|(command, _)| command)
            .collect()
    }

    /// The commands this package offers in root search, each with why it is
    /// unavailable on this system, if it is: first because the package does
    /// not support this system, else because the command does not.
    pub(crate) fn available_commands(&self) -> Vec<(CommandRegistration, Option<String>)> {
        let Ok(manifest) = &self.manifest else {
            return Vec::new();
        };
        let package = platform::unavailable(manifest.platforms.as_deref(), "this package");
        manifest
            .commands
            .iter()
            .map(|command| {
                let registration = CommandRegistration {
                    id: format!("{}#{}", self.identity.key(), command.id),
                    title: command.title.clone(),
                    subtitle: command
                        .subtitle
                        .clone()
                        .or_else(|| Some(manifest.title.clone())),
                    component: self.location.join(&command.component),
                    takes_query: command.takes_query,
                    search: command.search,
                };
                let unavailable = package.clone().or_else(|| {
                    platform::unavailable(command.platforms.as_deref(), "this command")
                });
                (registration, unavailable)
            })
            .collect()
    }
}

impl InstalledPackage {
    /// The commands of this package that compute root results and can run
    /// on this system; none if the package cannot be read.
    pub(crate) fn root_result_commands(&self) -> Vec<CommandRegistration> {
        let Ok(manifest) = &self.manifest else {
            return Vec::new();
        };
        self.available_commands()
            .into_iter()
            .zip(&manifest.commands)
            .filter(|((_, unavailable), command)| command.root_results && unavailable.is_none())
            .map(|((registration, _), _)| registration)
            .collect()
    }

    /// The commands of this package that supply root results ahead of the
    /// query and can run on this system; none if the package cannot be read.
    pub(crate) fn indexed_result_commands(&self) -> Vec<CommandRegistration> {
        let Ok(manifest) = &self.manifest else {
            return Vec::new();
        };
        self.available_commands()
            .into_iter()
            .zip(&manifest.commands)
            .filter(|((_, unavailable), command)| command.indexed_results && unavailable.is_none())
            .map(|((registration, _), _)| registration)
            .collect()
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct RegistryJson {
    version: u64,
    /// The next unused managed folder number.
    next: u64,
    packages: Vec<RecordJson>,
    /// Managed folders of replaced copies that could not be removed, such as
    /// a folder still in use on Windows; removal is tried again when Pane
    /// starts. Only folders listed here are ever removed that way: one the
    /// registry never recorded, as after it was lost, is left alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    leftovers: Vec<String>,
    /// Identities that are not installed but whose extension data Pane
    /// still keeps: the user uninstalled them keeping their saved data, or
    /// some of it could not be deleted. Installing the same source again
    /// uses that data and drops the record.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    retained: Vec<RetainedJson>,
}

#[derive(Clone, Serialize, Deserialize)]
struct RetainedJson {
    /// The source, the identity the data belongs to.
    #[serde(flatten)]
    source: Source,
    /// The package's title when it was uninstalled.
    title: String,
}

/// How to record that Pane keeps a removed package's data: under the title
/// it had, at a position among the retained records, or last.
struct Retain {
    title: String,
    at: Option<usize>,
}

/// Whether uninstalling a package keeps its saved data: its extension
/// settings and content. Its cache and local credentials are removed either
/// way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavedData {
    /// Keep them with the package identity, for when the same source is
    /// installed again.
    Keep,
    /// Delete them with the package.
    Delete,
}

/// Where a package identity stands in `installed.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Standing {
    Installed,
    /// Not installed, with its data kept.
    Retained,
    /// Neither installed nor with data on record.
    Neither,
}

/// Extension data Pane keeps for a package identity that is not installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetainedData {
    pub identity: PackageIdentity,
    /// The package's title when it was uninstalled.
    pub title: String,
}

/// What uninstalling left behind of the managed copy.
#[derive(Debug)]
pub(crate) enum Leftover {
    /// The managed copy is gone.
    None,
    /// The folder of the managed copy could not be removed, for this
    /// reason; Pane tries again when it next starts.
    Listed(PathBuf, String),
    /// The folder could not be removed, nor listed for removal at the next
    /// start.
    Unlisted(PathBuf, String),
}

#[derive(Clone, Serialize, Deserialize)]
struct RecordJson {
    /// The source, the package's identity: its local folder or npm name.
    #[serde(flatten)]
    source: Source,
    /// For a package from npm, the version installed and whether the user
    /// (or the dependency that installed it) pinned it to that version.
    #[serde(flatten, default, skip_serializing_if = "Option::is_none")]
    npm: Option<NpmRecordJson>,
    /// The managed folder under `packages/`.
    dir: String,
    /// Set when the user disabled the package; absent means enabled.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    disabled: bool,
    /// Set when Pane paused the package after it failed; absent means it
    /// runs. Separate from `disabled`, which is the user's choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paused: Option<PausedJson>,
    /// The identity each `local:` dependency its manifest declares resolved
    /// to when it was installed or updated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    dependencies: Vec<ResolvedJson>,
    /// Set when a component of its current code imports `wasi:http`;
    /// absent means none does (or it was installed before Pane recorded
    /// it, until it is reloaded or updated).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    network: bool,
}

/// An installed [`NpmPackage`] as its record writes it, beside the name its
/// source records: `"npm": "greeter", "npmVersion": "1.2.3", "pinned":
/// true`. (The name is the source's, so it is not written twice.)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct NpmRecordJson {
    #[serde(rename = "npmVersion")]
    version: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pinned: bool,
}

impl NpmRecordJson {
    fn of(package: &NpmPackage) -> NpmRecordJson {
        NpmRecordJson {
            version: package.version.clone(),
            pinned: package.pinned,
        }
    }

    /// The package this records for the package with `identity`, an npm
    /// one.
    fn package(&self, identity: &PackageIdentity) -> Option<NpmPackage> {
        Some(NpmPackage {
            name: identity.npm_name()?.to_owned(),
            version: self.version.clone(),
            pinned: self.pinned,
        })
    }
}

/// A dependency id and the source it resolved to.
#[derive(Clone, Serialize, Deserialize)]
struct ResolvedJson {
    id: String,
    #[serde(flatten)]
    source: Source,
}

/// The records of `package`'s dependencies, resolved from its source.
fn resolved_dependencies(package: &SourcePackage) -> Vec<ResolvedJson> {
    package
        .manifest
        .dependencies
        .iter()
        .filter_map(|dependency| {
            let PackageIdentity(source) = package.identity.dependency(&dependency.source).ok()?;
            Some(ResolvedJson {
                id: dependency.id.clone(),
                source,
            })
        })
        .collect()
}

#[derive(Clone, Serialize, Deserialize)]
struct PausedJson {
    #[serde(flatten)]
    pause: Pause,
    /// The managed folder of the code that failed. A pause recorded for
    /// other code (the folder changed) no longer applies.
    code: String,
}

/// Why Pane paused an installed package: it runs none of its code until the
/// user retries it, reloads or updates it, or disables it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Pause {
    pub after: PauseCause,
    /// The details: what failed, and how.
    pub why: String,
    /// The version of the package that failed, if its manifest has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Why a command of the paused package titled `title` does not run, or why
/// a call to it is refused; "The extension" when the title is not known.
pub(crate) fn paused_reason(title: &str) -> String {
    format!("{title} is paused after an error; retry it in Manage extensions")
}

/// What made Pane pause a package.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PauseCause {
    /// Its code could not start: a component could not be loaded or
    /// instantiated, or its reloaded code trapped as it started.
    FailedToStart,
    /// It crashed (trapped) too often.
    Crashes,
}

/// Pane's managed location for installed packages:
///
/// ```text
/// <dir>/installed.json      identities, their managed folders and whether
///                           each is disabled
/// <dir>/settings.json       each identity's extension settings
/// <dir>/packages/<n>/       one managed copy: pane.json and its components
/// ```
pub(crate) struct Store {
    dir: PathBuf,
    /// The registry, or why it could not be read. An unreadable registry is
    /// never overwritten.
    registry: Result<RegistryJson, String>,
}

impl Store {
    pub fn open(dir: PathBuf) -> Store {
        let registry = read_registry(&dir);
        let mut store = Store { dir, registry };
        store.remove_leftovers();
        store
    }

    /// Where `identity` stands in `installed.json` as it is on disk now,
    /// which another Pane on the same data folder may have changed since
    /// this one read it.
    pub fn standing_on_disk(&self, identity: &PackageIdentity) -> Result<Standing, PackageError> {
        let registry = read_registry(&self.dir).map_err(PackageError::Storage)?;
        let PackageIdentity(source) = identity;
        Ok(if registry.packages.iter().any(|r| &r.source == source) {
            Standing::Installed
        } else if registry.retained.iter().any(|r| &r.source == source) {
            Standing::Retained
        } else {
            Standing::Neither
        })
    }

    /// Tries again to remove the managed folders of replaced copies that
    /// could not be removed before, never one an installed package uses.
    /// Best effort: a folder that still cannot be removed stays listed.
    fn remove_leftovers(&mut self) {
        let Ok(registry) = &mut self.registry else {
            return;
        };
        if registry.leftovers.is_empty() {
            return;
        }
        let packages = self.dir.join(PACKAGES_DIR);
        let mut updated = registry.clone();
        updated.leftovers.retain(|dir| {
            let in_use = registry.packages.iter().any(|record| record.dir == *dir);
            if in_use {
                return false;
            }
            match fs::remove_dir_all(packages.join(dir)) {
                Ok(()) => false,
                Err(error) => error.kind() != io::ErrorKind::NotFound,
            }
        });
        if updated.leftovers.len() != registry.leftovers.len()
            && write_registry(&self.dir, &updated).is_ok()
        {
            *registry = updated;
        }
    }

    /// Why installed packages cannot be read, if they cannot.
    pub fn problem(&self) -> Option<String> {
        self.registry
            .as_ref()
            .err()
            .map(|reason| format!("Cannot read Pane's installed extensions: {reason}"))
    }

    /// Every installed package, from its managed manifest.
    pub fn installed(&self) -> Vec<InstalledPackage> {
        let Ok(registry) = &self.registry else {
            return Vec::new();
        };
        registry
            .packages
            .iter()
            .map(|record| {
                InstalledPackage::load(
                    PackageIdentity(record.source.clone()),
                    self.dir.join(PACKAGES_DIR).join(&record.dir),
                    !record.disabled,
                    record.network,
                    &record.dependencies,
                    record.npm.as_ref(),
                )
            })
            .collect()
    }

    pub fn is_installed(&self, identity: &PackageIdentity) -> bool {
        self.record(identity).is_some()
    }

    fn record(&self, identity: &PackageIdentity) -> Option<&RecordJson> {
        let PackageIdentity(source) = identity;
        self.registry
            .as_ref()
            .ok()?
            .packages
            .iter()
            .find(|record| &record.source == source)
    }

    /// Installs a package whose identity is not installed yet.
    pub fn install(&mut self, package: &SourcePackage) -> Result<InstalledPackage, PackageError> {
        if self.is_installed(&package.identity) {
            return Err(PackageError::AlreadyInstalled(package.identity.clone()));
        }
        self.write_copy(package, None, |_| {})
    }

    /// Replaces the managed copy of an installed package with the current
    /// contents of its source; the identity and its record stay the same.
    /// Once the new copy is recorded, `retire` is told the old copy's folder
    /// before it is removed: the old code must stop using it first (its
    /// helpers' programs, which Windows would otherwise keep in use).
    pub fn update(
        &mut self,
        package: &SourcePackage,
        retire: impl FnOnce(&Path),
    ) -> Result<InstalledPackage, PackageError> {
        let Some(record) = self.record(&package.identity) else {
            return Err(PackageError::NotInstalled(package.identity.clone()));
        };
        let old = record.dir.clone();
        self.write_copy(package, Some(old), retire)
    }

    /// Records whether each installed package of `identities` is enabled,
    /// in one write: all of them change, or, if one is not installed or the
    /// record cannot be written, none does. Their managed copies and
    /// settings are left as they are.
    pub fn set_enabled_all(
        &mut self,
        identities: &[PackageIdentity],
        enabled: bool,
    ) -> Result<(), PackageError> {
        let registry = self
            .registry
            .as_mut()
            .map_err(|reason| PackageError::Storage(reason.clone()))?;
        let mut updated = registry.clone();
        for identity in identities {
            let PackageIdentity(source) = identity;
            let Some(record) = updated.packages.iter_mut().find(|r| &r.source == source) else {
                return Err(PackageError::NotInstalled(identity.clone()));
            };
            record.disabled = !enabled;
            // Disabling or enabling ends a pause: the package starts afresh.
            record.paused = None;
        }
        write_registry(&self.dir, &updated)
            .map_err(|error| PackageError::Storage(error.to_string()))?;
        *registry = updated;
        Ok(())
    }

    /// The installed packages Pane paused, each with why, as recorded for
    /// their current code.
    pub fn paused(&self) -> Vec<(PackageIdentity, Pause)> {
        let Ok(registry) = &self.registry else {
            return Vec::new();
        };
        registry
            .packages
            .iter()
            .filter_map(|record| {
                let paused = record.paused.as_ref()?;
                (paused.code == record.dir)
                    .then(|| (PackageIdentity(record.source.clone()), paused.pause.clone()))
            })
            .collect()
    }

    /// Records that Pane paused the installed package with `identity` for
    /// `pause`, or, with `None`, that it runs again.
    pub fn set_paused(
        &mut self,
        identity: &PackageIdentity,
        pause: Option<Pause>,
    ) -> Result<(), PackageError> {
        let registry = self
            .registry
            .as_mut()
            .map_err(|reason| PackageError::Storage(reason.clone()))?;
        let PackageIdentity(source) = identity;
        let mut updated = registry.clone();
        let Some(record) = updated.packages.iter_mut().find(|r| &r.source == source) else {
            return Err(PackageError::NotInstalled(identity.clone()));
        };
        let paused = pause.map(|pause| PausedJson {
            pause,
            code: record.dir.clone(),
        });
        if paused.is_none() && record.paused.is_none() {
            return Ok(());
        }
        record.paused = paused;
        write_registry(&self.dir, &updated)
            .map_err(|error| PackageError::Storage(error.to_string()))?;
        *registry = updated;
        Ok(())
    }

    /// The identities that are not installed but whose extension data Pane
    /// keeps, in the order they were uninstalled.
    pub fn retained(&self) -> Vec<RetainedData> {
        let Ok(registry) = &self.registry else {
            return Vec::new();
        };
        registry
            .retained
            .iter()
            .map(|record| RetainedData {
                identity: PackageIdentity(record.source.clone()),
                title: record.title.clone(),
            })
            .collect()
    }

    /// Removes again the package with `identity` that an install added,
    /// putting back, at its place among them, the record that Pane kept its
    /// data under `title` if it had one (`(title, index)`).
    pub(crate) fn undo_install(
        &mut self,
        identity: &PackageIdentity,
        retained: Option<(String, usize)>,
    ) -> Result<Leftover, PackageError> {
        let retain = retained.map(|(title, at)| Retain {
            title,
            at: Some(at),
        });
        self.remove(identity, retain)
    }

    /// Uninstalls the packages of `removals`, in one write: each record
    /// goes, and with a title, its identity is recorded as keeping extension
    /// data under that title. `installed.json` is read again first and only
    /// those records change in it, so what another Pane on the same data
    /// folder recorded since is kept. If one is not installed or the record
    /// cannot be written, nothing changes. Then each managed copy is removed; a
    /// managed folder that cannot be (one in use on Windows) is listed so
    /// that the next start removes it, as an update's replaced copy is.
    /// Returns what is left of each managed copy, in the order given.
    pub fn uninstall_all(
        &mut self,
        removals: &[(PackageIdentity, Option<String>)],
    ) -> Result<Vec<Leftover>, PackageError> {
        let removals: Vec<(PackageIdentity, Option<Retain>)> = removals
            .iter()
            .map(|(identity, title)| {
                let retain = title.clone().map(|title| Retain { title, at: None });
                (identity.clone(), retain)
            })
            .collect();
        self.remove_all(&removals)
    }

    /// Uninstalls one package as [`Store::uninstall_all`] does, recording
    /// retained data under a title, at a position among the retained records
    /// if given, else last.
    fn remove(
        &mut self,
        identity: &PackageIdentity,
        retain: Option<Retain>,
    ) -> Result<Leftover, PackageError> {
        let mut left = self.remove_all(&[(identity.clone(), retain)])?;
        Ok(left.pop().unwrap_or(Leftover::None))
    }

    /// Uninstalls as [`Store::uninstall_all`] does, each retained record
    /// under a title at a position among them if given, else last.
    fn remove_all(
        &mut self,
        removals: &[(PackageIdentity, Option<Retain>)],
    ) -> Result<Vec<Leftover>, PackageError> {
        let registry = self
            .registry
            .as_mut()
            .map_err(|reason| PackageError::Storage(reason.clone()))?;
        // The same changes to the records on disk, which another Pane may
        // have changed since, and to this Pane's.
        let mut on_disk = read_registry(&self.dir).map_err(PackageError::Storage)?;
        let mut updated = registry.clone();
        let mut dirs = Vec::new();
        for (identity, retain) in removals {
            let PackageIdentity(local) = identity;
            let missing = || PackageError::NotInstalled(identity.clone());
            take_record(&mut updated, local).ok_or_else(missing)?;
            // Its managed copy as recorded now: another Pane may have
            // updated it since.
            let record = take_record(&mut on_disk, local).ok_or_else(missing)?;
            dirs.push(record.dir);
            if let Some(Retain { title, at }) = retain {
                put_retained(&mut updated, local, title.clone(), *at);
                put_retained(&mut on_disk, local, title.clone(), *at);
            }
        }
        write_registry(&self.dir, &on_disk)
            .map_err(|error| PackageError::Storage(error.to_string()))?;
        *registry = updated;
        let failed: Vec<Option<(String, PathBuf, String)>> = dirs
            .into_iter()
            .map(|dir| {
                let location = self.dir.join(PACKAGES_DIR).join(&dir);
                match fs::remove_dir_all(&location) {
                    Ok(()) => None,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                    Err(error) => Some((dir, location, error.to_string())),
                }
            })
            .collect();
        let left: Vec<String> = failed
            .iter()
            .flatten()
            .map(|(dir, ..)| dir.clone())
            .collect();
        let recorded = left.is_empty() || {
            on_disk.leftovers.extend(left.iter().cloned());
            let written = write_registry(&self.dir, &on_disk).is_ok();
            if written {
                registry.leftovers.extend(left);
            }
            written
        };
        Ok(failed
            .into_iter()
            .map(|failed| match failed {
                None => Leftover::None,
                Some((_, location, error)) if recorded => Leftover::Listed(location, error),
                Some((_, location, error)) => Leftover::Unlisted(location, error),
            })
            .collect())
    }

    /// Drops the record that Pane keeps extension data for `identity`, once
    /// that data is deleted or no longer kept. `installed.json` is read
    /// again first and only that record is removed from it, so what another
    /// Pane on the same data folder recorded since, such as installing the
    /// same source again, is kept. A failure leaves the record.
    pub fn forget_retained(&mut self, identity: &PackageIdentity) -> Result<(), PackageError> {
        let registry = self
            .registry
            .as_mut()
            .map_err(|reason| PackageError::Storage(reason.clone()))?;
        let PackageIdentity(source) = identity;
        let mut on_disk = read_registry(&self.dir).map_err(PackageError::Storage)?;
        if on_disk
            .retained
            .iter()
            .any(|record| &record.source == source)
        {
            on_disk.retained.retain(|record| &record.source != source);
            write_registry(&self.dir, &on_disk)
                .map_err(|error| PackageError::Storage(error.to_string()))?;
        }
        registry.retained.retain(|record| &record.source != source);
        Ok(())
    }

    /// Records that Pane keeps extension data for `identity`, which is not
    /// installed, under `title`.
    pub fn retain(
        &mut self,
        identity: &PackageIdentity,
        title: String,
    ) -> Result<(), PackageError> {
        let registry = self
            .registry
            .as_mut()
            .map_err(|reason| PackageError::Storage(reason.clone()))?;
        let PackageIdentity(local) = identity;
        let mut updated = registry.clone();
        put_retained(&mut updated, local, title, None);
        write_registry(&self.dir, &updated)
            .map_err(|error| PackageError::Storage(error.to_string()))?;
        *registry = updated;
        Ok(())
    }

    /// Copies the package into a fresh managed folder, then records it,
    /// replacing `old`'s folder if given. A failure leaves the previous state.
    fn write_copy(
        &mut self,
        package: &SourcePackage,
        old: Option<String>,
        retire: impl FnOnce(&Path),
    ) -> Result<InstalledPackage, PackageError> {
        let registry = self
            .registry
            .as_mut()
            .map_err(|reason| PackageError::Storage(reason.clone()))?;
        let storage = |error: io::Error| PackageError::Storage(error.to_string());
        // A folder can exist at or beyond `next` if the registry was lost or
        // replaced; it is skipped, never deleted.
        let mut number = registry.next;
        let (dir, location) = loop {
            let dir = number.to_string();
            let location = self.dir.join(PACKAGES_DIR).join(&dir);
            match fs::symlink_metadata(&location) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => break (dir, location),
                Err(error) => return Err(storage(error)),
                Ok(_) => number += 1,
            }
        };
        let copied = copy_package(package, &location);
        if let Err(error) = copied {
            let _ = fs::remove_dir_all(&location);
            return Err(storage(error));
        }
        let PackageIdentity(local) = &package.identity;
        let mut updated = RegistryJson {
            next: number + 1,
            ..registry.clone()
        };
        let dependencies = resolved_dependencies(package);
        let npm = package
            .npm
            .as_ref()
            .map(|origin| NpmRecordJson::of(&origin.package));
        // An update keeps the record, so a disabled package stays disabled.
        let enabled = match updated.packages.iter_mut().find(|r| &r.source == local) {
            Some(record) => {
                record.dir = dir;
                // New code has not failed.
                record.paused = None;
                record.dependencies = dependencies.clone();
                record.npm = npm.clone();
                record.network = package.network;
                !record.disabled
            }
            None => {
                // Data kept from an earlier installation of this identity
                // is its own again.
                updated.retained.retain(|record| &record.source != local);
                updated.packages.push(RecordJson {
                    source: local.clone(),
                    npm: npm.clone(),
                    dir,
                    disabled: false,
                    paused: None,
                    dependencies: dependencies.clone(),
                    network: package.network,
                });
                true
            }
        };
        if let Err(error) = write_registry(&self.dir, &updated) {
            let _ = fs::remove_dir_all(&location);
            return Err(storage(error));
        }
        *registry = updated;
        if let Some(old) = &old {
            retire(&self.dir.join(PACKAGES_DIR).join(old));
        }
        if let Some(old) = old
            && fs::remove_dir_all(self.dir.join(PACKAGES_DIR).join(&old)).is_err()
        {
            // A folder still in use (Windows) is left behind, and listed so
            // that the next start removes it. Best effort: if that cannot be
            // recorded, the folder stays.
            let mut listed = registry.clone();
            listed.leftovers.push(old);
            if write_registry(&self.dir, &listed).is_ok() {
                *registry = listed;
            }
        }
        Ok(InstalledPackage::load(
            package.identity.clone(),
            location,
            enabled,
            package.network,
            &dependencies,
            npm.as_ref(),
        ))
    }
}

/// Records in `registry` that data is kept for the local source `local`,
/// last uninstalled as `title`: at position `at` among the records if
/// given, else last.
/// Removes and returns the record of the installed package from `local`.
fn take_record(registry: &mut RegistryJson, source: &Source) -> Option<RecordJson> {
    let index = registry.packages.iter().position(|r| r.source == *source)?;
    Some(registry.packages.remove(index))
}

fn put_retained(registry: &mut RegistryJson, source: &Source, title: String, at: Option<usize>) {
    registry.retained.retain(|record| record.source != *source);
    let record = RetainedJson {
        source: source.clone(),
        title,
    };
    match at {
        Some(at) if at <= registry.retained.len() => registry.retained.insert(at, record),
        _ => registry.retained.push(record),
    }
}

/// Writes the validated `pane.json` and copies the components it names and
/// the file of each helper for this system, keeping their relative paths.
/// Nothing else in the source folder is copied, not even helpers' files for
/// other systems.
fn copy_package(package: &SourcePackage, location: &Path) -> io::Result<()> {
    fs::create_dir_all(location)?;
    fs::write(location.join(MANIFEST_FILE), &package.manifest_text)?;
    let helper_files: Vec<&Path> = package
        .manifest
        .helpers
        .iter()
        .filter_map(ManifestHelper::for_this_system)
        .collect();
    for file in package
        .manifest
        .components()
        .map(|(_, component)| component)
        .chain(helper_files.iter().copied())
    {
        let source = package.folder.join(file);
        let target = location.join(file);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        // Checked when the package was read; a helper file replaced by a
        // link since is not followed.
        let is_helper = helper_files.contains(&file);
        if is_helper && !fs::symlink_metadata(&source)?.is_file() {
            return Err(io::Error::other(format!(
                "the helper file {} is no longer a regular file",
                file.display()
            )));
        }
        if is_helper {
            // Closed and renamed into place rather than written straight to
            // `target`: a helper run soon after this install (or a reinstall
            // rewriting it while another generation is spawning it) must
            // never see it half-written and get Linux's `ETXTBSY`.
            runner::copy_executable(&source, &target)?;
        } else {
            fs::copy(source, target)?;
        }
    }
    for file in helper_files {
        make_executable(&location.join(file))?;
    }
    Ok(())
}

/// Lets the system run the helper file at `path` (a package fetched as an
/// archive or through a tool may have lost the execute permission), with
/// exactly `rwxr-xr-x`: no set-user-id, set-group-id or sticky bit, and no
/// one but its owner may change it.
#[cfg(unix)]
fn make_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Reads the registry in `dir`; a missing one holds nothing.
fn read_registry(dir: &Path) -> Result<RegistryJson, String> {
    match fs::read_to_string(dir.join(REGISTRY_FILE)) {
        Ok(text) => serde_json::from_str::<RegistryJson>(&text)
            .map_err(|error| error.to_string())
            .and_then(|registry| {
                if registry.version == REGISTRY_VERSION {
                    Ok(registry)
                } else {
                    Err(format!(
                        "{REGISTRY_FILE} has version {}, this Pane reads {REGISTRY_VERSION}",
                        registry.version
                    ))
                }
            }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(RegistryJson {
            version: REGISTRY_VERSION,
            next: 1,
            packages: Vec::new(),
            leftovers: Vec::new(),
            retained: Vec::new(),
        }),
        Err(error) => Err(error.to_string()),
    }
    .map_err(|reason| format!("{}: {reason}", dir.join(REGISTRY_FILE).display()))
}

/// Replaces the registry whole (see [`write_atomically`] for what a crash or
/// a second Pane process can do to it).
fn write_registry(dir: &Path, registry: &RegistryJson) -> io::Result<()> {
    let text = serde_json::to_string_pretty(registry).map_err(io::Error::other)?;
    write_atomically(&dir.join(REGISTRY_FILE), text.as_bytes(), Readers::Default)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A package folder in `dir` with one command and, for this system, a
    /// helper `tool` whose file holds `helper`.
    fn package_with_helper(dir: &Path, helper: &[u8]) -> PathBuf {
        let target = Target::current().expect("Pane names this system's target");
        let file = format!("helpers/tool{}", target.exe_suffix());
        let folder = dir.join("source");
        fs::create_dir_all(folder.join("helpers")).unwrap();
        fs::write(folder.join("command.wasm"), b"not checked here").unwrap();
        fs::write(folder.join(&file), helper).unwrap();
        let manifest = format!(
            r#"{{ "manifestVersion": 1, "title": "Tool", "apiVersion": "0.1",
                 "commands": [{{ "id": "c", "title": "C", "component": "command.wasm" }}],
                 "helpers": [{{ "id": "tool", "targets": {{ "{}": "{file}" }} }}] }}"#,
            target.id()
        );
        fs::write(folder.join(MANIFEST_FILE), manifest).unwrap();
        folder
    }

    #[test]
    fn records_of_local_and_npm_packages_are_read_back() {
        // As #42 wrote them, and one from npm with its pinned version and a
        // dependency on another npm package.
        let text = r#"{
            "version": 1, "next": 3,
            "packages": [
                { "local": "/src/a", "dir": "1", "dependencies": [{ "id": "b", "local": "/src/b" }] },
                { "npm": "@pane-samples/greeter", "npmVersion": "0.1.0", "pinned": true, "dir": "2",
                  "dependencies": [{ "id": "c", "npm": "c" }] }
            ],
            "retained": [{ "npm": "gone", "title": "Gone" }, { "local": "/src/x", "title": "X" }]
        }"#;
        let registry: RegistryJson = serde_json::from_str(text).unwrap();
        let [a, greeter] = registry.packages.as_slice() else {
            panic!("two records")
        };
        assert_eq!(
            a.source,
            Source::Local {
                local: "/src/a".into()
            }
        );
        assert_eq!(a.npm, None);
        assert_eq!(
            a.dependencies[0].source,
            Source::Local {
                local: "/src/b".into()
            }
        );
        assert_eq!(
            greeter.source,
            Source::Npm {
                npm: "@pane-samples/greeter".into()
            }
        );
        assert_eq!(
            greeter.npm,
            Some(NpmRecordJson {
                version: "0.1.0".into(),
                pinned: true
            })
        );
        assert_eq!(
            greeter.dependencies[0].source,
            Source::Npm { npm: "c".into() }
        );
        assert_eq!(
            registry.retained[0].source,
            Source::Npm { npm: "gone".into() }
        );

        let written = serde_json::to_value(&registry).unwrap();
        assert_eq!(
            written["packages"][1],
            serde_json::json!({
                "npm": "@pane-samples/greeter", "npmVersion": "0.1.0", "pinned": true, "dir": "2",
                "dependencies": [{ "id": "c", "npm": "c" }]
            })
        );
        assert_eq!(
            written["packages"][0],
            serde_json::json!({
                "local": "/src/a", "dir": "1", "dependencies": [{ "id": "b", "local": "/src/b" }]
            })
        );
    }

    /// The manifest of a package whose one command has `schedule`, as JSON.
    fn scheduled(schedule: &str) -> Result<Manifest, PackageError> {
        Manifest::parse(&format!(
            r#"{{ "manifestVersion": 1, "title": "Ticks", "apiVersion": "0.1",
                 "commands": [{{ "id": "tick", "title": "Tick", "component": "c.wasm",
                                 "schedule": {schedule} }}] }}"#
        ))
    }

    #[test]
    fn a_schedule_runs_every_so_many_minutes_within_a_day() {
        let manifest = scheduled(r#"{ "everyMinutes": 15 }"#).unwrap();
        let schedule = manifest.commands[0].schedule.unwrap();
        assert_eq!(schedule.every(), std::time::Duration::from_secs(15 * 60));
        assert!(manifest.exports_of(Path::new("c.wasm")).scheduled_task);
        assert_eq!(schedule.to_string(), "every 15 minutes");
        assert_eq!(
            scheduled(r#"{ "everyMinutes": 1 }"#).unwrap().commands[0]
                .schedule
                .unwrap()
                .to_string(),
            "every minute"
        );
        assert_eq!(
            scheduled(r#"{ "everyMinutes": 120 }"#).unwrap().commands[0]
                .schedule
                .unwrap()
                .to_string(),
            "every 2 hours"
        );
        scheduled(r#"{ "everyMinutes": 1440 }"#).unwrap();

        for (schedule, problem) in [
            (
                r#"{ "everyMinutes": 0 }"#,
                "the schedule of command `tick` runs every 0 minutes; it must be from 1 to 1440 (a day)",
            ),
            (
                r#"{ "everyMinutes": 1441 }"#,
                "the schedule of command `tick` runs every 1441 minutes; it must be from 1 to 1440 (a day)",
            ),
        ] {
            assert_eq!(
                scheduled(schedule).unwrap_err().to_string(),
                PackageError::InvalidManifest(problem.into()).to_string()
            );
        }
        // One kind of schedule: anything else is refused, not ignored.
        assert!(scheduled(r#"{ "cron": "* * * * *" }"#).is_err());
        assert!(scheduled(r#"{ "everyMinutes": 5, "at": "09:00" }"#).is_err());
        assert!(scheduled(r#""15m""#).is_err());
        // A command without one has none, and exports nothing for it.
        let plain = Manifest::parse(
            r#"{ "manifestVersion": 1, "title": "Plain", "apiVersion": "0.1",
                 "commands": [{ "id": "c", "title": "C", "component": "c.wasm" }] }"#,
        )
        .unwrap();
        assert_eq!(plain.commands[0].schedule, None);
        assert!(!plain.exports_of(Path::new("c.wasm")).scheduled_task);
    }

    /// This test binary: a program for this system's target.
    fn a_program() -> Vec<u8> {
        fs::read(std::env::current_exe().unwrap()).unwrap()
    }

    #[test]
    fn an_update_retires_the_old_copy_before_removing_it() {
        let dir = tempfile::tempdir().unwrap();
        let folder = package_with_helper(dir.path(), &a_program());
        let mut store = Store::open(dir.path().join("pane"));
        let first = store
            .install(&SourcePackage::read(&folder).unwrap())
            .unwrap();

        let mut retired = None;
        let second = store
            .update(&SourcePackage::read(&folder).unwrap(), |old| {
                // Still there: whatever runs from it is stopped first.
                assert!(old.join(MANIFEST_FILE).is_file());
                retired = Some(old.to_path_buf());
            })
            .unwrap();

        assert_eq!(retired.as_deref(), Some(first.location.as_path()));
        assert!(!first.location.exists());
        assert!(second.location.join(MANIFEST_FILE).is_file());
    }

    #[cfg(unix)]
    #[test]
    fn an_installed_helper_is_exactly_rwxr_xr_x() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let folder = package_with_helper(dir.path(), &a_program());
        let helper = folder.join("helpers/tool");
        // Set-user-id, set-group-id, sticky, and writable by anyone.
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o7777)).unwrap();
        let mut store = Store::open(dir.path().join("pane"));

        let installed = store
            .install(&SourcePackage::read(&folder).unwrap())
            .unwrap();

        let mode = fs::metadata(installed.location.join("helpers/tool"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o7777, 0o755, "{mode:o}");
    }

    #[cfg(unix)]
    #[test]
    fn a_helper_file_that_is_a_link_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let folder = package_with_helper(dir.path(), &a_program());
        let real = dir.path().join("real");
        fs::rename(folder.join("helpers/tool"), &real).unwrap();
        std::os::unix::fs::symlink(&real, folder.join("helpers/tool")).unwrap();

        let error = SourcePackage::read(&folder).unwrap_err();

        assert_eq!(
            error.to_string(),
            format!(
                "Not ready to run: the package ships helper `tool` for {}, but its file \
                 helpers/tool is a symbolic link; a helper must be a regular file in the \
                 package",
                Target::current().unwrap()
            )
        );
    }
}
