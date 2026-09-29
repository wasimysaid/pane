//! Uninstalling an installed package, or one with the installed packages that
//! require it (see `dependents`), with the user's explicit choice to keep or
//! delete their saved data (their extension settings and content). Several
//! packages are uninstalled as one would be, their records removed in the
//! same write.
//!
//! Choosing a row of the confirmation applies at once in this launcher: the
//! package leaves root search, the extension list and the operation
//! targets, its instances stop, an open command of it closes and it can no
//! longer save data. Then Pane, never the package's code:
//!
//! 1. records it as uninstalled in `installed.json`, with a record of the
//!    identity whose saved data is kept when the user keeps it. If this
//!    fails nothing else changes and the package is back as it was. This is
//!    the point after which the package is uninstalled;
//! 2. removes its managed copy. A folder still in use (Windows) is listed
//!    for removal at the next start, as an update's replaced copy is;
//! 3. removes its cache and local credentials, and its settings and content
//!    too when the user deletes them. Data that could not be removed stays
//!    recorded as kept, so it is not lost track of.
//!
//! Recording first means a failure leaves either an installed package with
//! all its data or an uninstalled one whose leftovers are on record, never
//! an installed package whose data was deleted. A failure of step 2 or 3 is
//! explained rather than reported as a
//! successful uninstall. The source folder and anything outside Pane's data
//! folder are never touched. Kept data belongs to the package identity:
//! installing the same source again finds it, another source never does.

use std::future::Future;
use std::path::PathBuf;

use super::{
    Changing, Entry, Launcher, LauncherView, Question, Row, Screen, State, Status, dependents,
    off_thread,
};
use crate::extension_data::{DataKind, ExtensionData};
use crate::packages::{
    InstalledPackage, Leftover, PackageError, PackageIdentity, Pause, SavedData,
};
use crate::platform;

/// An uninstall begun by [`Launcher::begin_uninstall`]: the packages,
/// already removed from the launcher, the one asked about first, and where
/// each was, to put them back if their uninstall cannot be recorded.
pub(super) struct Uninstall {
    removed: Vec<Removed>,
    saved: SavedData,
}

/// A package removed from the launcher to be uninstalled.
struct Removed {
    package: InstalledPackage,
    /// Its place among the installed packages when it was removed, after
    /// those removed before it.
    index: usize,
    /// Why Pane had paused it, if it had.
    pause: Option<Pause>,
}

impl Launcher {
    /// Uninstalls the installed package with `identity`, keeping or deleting
    /// its saved data (see the module documentation). Its cache and local
    /// credentials are removed either way; its source folder is not touched
    /// and none of its code runs. Await the returned future for the outcome.
    /// It uninstalls this package alone, without asking about the packages
    /// that require it (the extension list asks).
    ///
    /// While the package is being reloaded, updated or uninstalled, this
    /// explains why it does nothing.
    pub fn uninstall(
        &self,
        identity: &PackageIdentity,
        saved: SavedData,
    ) -> impl Future<Output = ()> + Send + 'static {
        let mut state = self.lock();
        let uninstall = self.begin_uninstall(&mut state, vec![identity.clone()], saved);
        let epoch = state.screen_epoch;
        drop(state);
        let launcher = self.clone();
        async move {
            if let Some(uninstall) = uninstall {
                launcher.finish_uninstall(epoch, uninstall).await;
            }
        }
    }

    /// Asks whether to uninstall the installed package with `identity`,
    /// saying what is removed, what saved data it has and what is never
    /// touched, with a choice to keep or delete that data.
    pub(super) fn show_uninstall(&self, state: &mut State, identity: &PackageIdentity) {
        let Some(installation) = &self.installation else {
            return;
        };
        let title = state.title_of(identity);
        let choice = |id: &str, title: &str, subtitle: &str| Row {
            id: id.into(),
            title: title.into(),
            subtitle: Some(subtitle.into()),
            unavailable: None,
        };
        state.next_screen();
        state.entries = vec![
            Entry::Uninstall(identity.clone(), SavedData::Keep),
            Entry::Uninstall(identity.clone(), SavedData::Delete),
            Entry::Cancel,
        ];
        let source = match identity.local_folder() {
            Some(folder) => format!("Its source folder {}", folder.display()),
            None => "Its source".into(),
        };
        let details = vec![
            format!("From {identity}"),
            "Pane removes its installed copy, its cache and its credentials on this computer, \
             and the extension does not run. Deleting a credential does not sign you out of an \
             online service."
                .into(),
            saved_data(&installation.data, identity),
            format!("{source} and files it saved elsewhere are not touched."),
        ];
        let screen = Screen::Confirm {
            question: Question::Uninstall(identity.clone()),
            details,
        };
        state.view = LauncherView::new(screen, format!("Uninstall {title}?")).with_rows(vec![
            choice(
                "keep",
                "Uninstall and keep saved data",
                "Keep its settings and content; installing it again from the same source \
                 restores them",
            ),
            choice(
                "delete",
                "Uninstall and delete saved data",
                "Delete its settings and content too",
            ),
            choice("cancel", "Cancel", "Keep it installed"),
        ]);
    }

    /// Removes the packages with `identities` (the one asked about first)
    /// from this launcher at once, to be uninstalled together by
    /// [`Launcher::finish_uninstall`]: they leave root search, the extension
    /// list and the operation targets, their instances stop, an open command
    /// of one closes and they can no longer save data. Removes none of them,
    /// explains why and returns `None` if one is not installed or something
    /// else is happening to it.
    pub(super) fn begin_uninstall(
        &self,
        state: &mut State,
        identities: Vec<PackageIdentity>,
        saved: SavedData,
    ) -> Option<Uninstall> {
        let Some(installation) = &self.installation else {
            let error = PackageError::Storage("this launcher does not install packages".into());
            state.view.status = Status::Error(error.to_string());
            return None;
        };
        if let Some(missing) = identities.iter().find(|i| state.package(i).is_none()) {
            let error = PackageError::NotInstalled(missing.clone());
            state.view.status = Status::Error(error.to_string());
            return None;
        }
        let claims: Vec<(PackageIdentity, Changing)> = identities
            .iter()
            .map(|identity| (identity.clone(), Changing::Uninstalling))
            .collect();
        match state.claim_all(&claims) {
            Ok(()) => {}
            // As when enabling or disabling it is being recorded: pressing
            // Enter twice.
            Err((_, Changing::Recording)) => return None,
            Err((identity, busy)) => {
                let message = format!("{} {}", state.title_of(&identity), busy.doing());
                state.view.status = Status::Error(message);
                return None;
            }
        }
        let mut removed = Vec::new();
        let mut components: Vec<PathBuf> = Vec::new();
        for identity in &identities {
            let index = state
                .packages
                .iter()
                .position(|p| p.identity == *identity)
                .expect("checked that each is installed");
            let package = state.packages.remove(index);
            installation.data.uninstall(identity);
            // Its development ends, with a build that is running.
            self.developing.end(Some(identity));
            let commands: Vec<PathBuf> = package
                .commands()
                .into_iter()
                .map(|command| command.component)
                .collect();
            if let Ok(runtime) = self.runtime() {
                // Operation components too: every instance of its managed
                // copy.
                let mut all = commands.clone();
                if let Ok(manifest) = &package.manifest {
                    all.extend(
                        manifest
                            .operations
                            .iter()
                            .map(|operation| package.location.join(&operation.component)),
                    );
                }
                runtime.forget(all);
            }
            components.extend(commands);
            // Its pause goes with it, in memory only: if the uninstall cannot
            // be recorded, it is put back, as `installed.json` still has it.
            let pause = state.paused.forget(identity);
            removed.push(Removed {
                package,
                index,
                pause,
            });
        }
        // Their hotkeys are released now, and forgotten once they are
        // uninstalled.
        self.sync_hotkeys(state);
        // Their results kept for root search go, and so does an answer from
        // one being awaited.
        Launcher::forget_indexes(state);
        let open = state
            .open
            .as_ref()
            .is_some_and(|open| components.contains(open));
        if open {
            self.show_root(state, None);
        } else {
            self.refresh(state);
        }
        state.view.status = Status::Running;
        Some(Uninstall { removed, saved })
    }

    /// Uninstalls the packages removed by [`Launcher::begin_uninstall`] from
    /// Pane's files, recorded in one write and putting all of them back if
    /// that cannot be recorded, and shows the outcome: what could not be
    /// removed afterwards is named with the package it belongs to.
    pub(super) async fn finish_uninstall(&self, epoch: u64, uninstall: Uninstall) {
        let Uninstall { removed, saved } = uninstall;
        let identities: Vec<PackageIdentity> = removed
            .iter()
            .map(|removed| removed.package.identity.clone())
            .collect();
        let titles: Vec<String> = removed.iter().map(|r| r.package.title()).collect();
        let installation = self
            .installation
            .clone()
            .expect("begin_uninstall checked there is an installation");
        // The runtime has dropped their instances before their files go;
        // their pending calls were stopped when their generations ended.
        if let Ok(runtime) = self.runtime() {
            runtime.running().await;
        }
        let keeps = |kind, identity| installation.data.count(kind, identity) != Ok(0);
        let removals: Vec<(PackageIdentity, Option<String>)> = removed
            .iter()
            .map(|removed| {
                let identity = &removed.package.identity;
                let retain = (saved == SavedData::Keep
                    && (keeps(DataKind::Settings, identity) || keeps(DataKind::Content, identity)))
                .then(|| removed.package.title());
                (identity.clone(), retain)
            })
            .collect();
        let recorded = {
            let store = installation.store.clone();
            off_thread(move || {
                let mut store = store.lock().unwrap_or_else(|p| p.into_inner());
                store.uninstall_all(&removals)
            })
            .await
        };
        let leftovers = match recorded {
            Err(error) => {
                let mut state = self.lock();
                // Put back in the reverse order of their removal, each at the
                // place it had then.
                for Removed {
                    package,
                    index,
                    pause,
                } in removed.into_iter().rev()
                {
                    let identity = package.identity.clone();
                    let enabled = package.enabled;
                    let at = index.min(state.packages.len());
                    state.packages.insert(at, package);
                    installation.data.reinstate(&identity, enabled);
                    // Still paused, as `installed.json` still records it.
                    if let Some(pause) = pause {
                        installation.data.pause(&identity);
                        state.paused.restore(identity, pause);
                    }
                }
                self.sync_hotkeys(&mut state);
                let message = match titles.as_slice() {
                    [title] => format!(
                        "Could not uninstall {title}: {error}. It is still installed and \
                         nothing was deleted."
                    ),
                    _ => format!(
                        "Could not uninstall {}: {error}. They are all still installed and \
                         nothing was deleted.",
                        platform::join(&titles)
                    ),
                };
                self.end_uninstall(&mut state, epoch, &identities, Status::Error(message));
                return;
            }
            Ok(leftovers) => leftovers,
        };
        let mut problems = Vec::new();
        for (identity, title) in identities.iter().zip(&titles) {
            let forget_hotkeys = self.forget_hotkeys_of(&mut self.lock(), identity);
            let forget_aliases = self.forget_aliases_of(&mut self.lock(), identity);
            let forget_tasks = self.forget_tasks_of(&mut self.lock(), identity);
            let forget_services = self.forget_services_of(&mut self.lock(), identity);
            let files = self.lock().files.clone();
            let data = installation.data.clone();
            let store = installation.store.clone();
            let identity = identity.clone();
            let title = title.clone();
            let (found, retained) = off_thread(move || {
                let mut problems = data.remove_uninstalled(&identity, saved);
                if let Some(Err(error)) = forget_hotkeys.map(|forget| forget()) {
                    problems.push(format!("could not forget its hotkeys: {error}"));
                }
                if let Some(Err(error)) = forget_aliases.map(|forget| forget()) {
                    problems.push(format!("could not forget its aliases: {error}"));
                }
                if let Some(Err(error)) = forget_tasks.map(|forget| forget()) {
                    problems.push(format!("could not forget its schedules: {error}"));
                }
                if let Some(Err(error)) = forget_services.map(|forget| forget()) {
                    problems.push(format!("could not forget its services: {error}"));
                }
                // The folder it was granted is Pane's record, not its data:
                // it goes whether or not data is kept, for every package
                // uninstalled together.
                if let Some(files) = files
                    && files.granted(&identity.key()).is_some()
                    && let Err(error) = files.revoke(&identity.key())
                {
                    problems.push(format!("could not forget its folder: {error}"));
                }
                let store = &mut store.lock().unwrap_or_else(|p| p.into_inner());
                let recorded = store.retained().iter().any(|r| r.identity == identity);
                if !recorded
                    && data.holds_any(&identity)
                    && let Err(error) = store.retain(&identity, title)
                {
                    problems.push(format!(
                        "could not record that some of its data is kept: {error}"
                    ));
                }
                (problems, store.retained())
            })
            .await;
            self.lock().retained = retained;
            problems.push(found);
        }
        for (found, left) in problems.iter_mut().zip(leftovers) {
            match left {
                Leftover::None => {}
                Leftover::Listed(path, error) => found.push(format!(
                    "its installed copy in {} could not be removed yet ({error}); Pane removes \
                     it when it next starts",
                    path.display()
                )),
                Leftover::Unlisted(path, error) => found.push(format!(
                    "its installed copy in {} could not be removed ({error}); delete that \
                     folder yourself",
                    path.display()
                )),
            }
        }
        let status = outcome(&titles, saved, &problems);
        let mut state = self.lock();
        self.end_uninstall(&mut state, epoch, &identities, status);
    }

    /// Ends an uninstall of the packages with `identities` (the one asked
    /// about first) with `status`, shown if the screen is still the one of
    /// `epoch`: from its confirmation, on the extension list.
    fn end_uninstall(
        &self,
        state: &mut State,
        epoch: u64,
        identities: &[PackageIdentity],
        status: Status,
    ) {
        for identity in identities {
            state.release(identity);
        }
        if state.screen_epoch != epoch {
            self.refresh(state);
            return;
        }
        let first = &identities[0];
        match &state.view.screen {
            Screen::Confirm {
                question: Question::Uninstall(asked) | Question::UninstallDependents(asked),
                ..
            } if asked == first => {
                let identity = first.clone();
                self.show_extensions_at(
                    state,
                    |entry| matches!(entry, Entry::AskUninstall(asked) if *asked == identity),
                );
            }
            _ => self.refresh(state),
        }
        state.view.status = status;
    }
}

/// "Saved data: …": how many settings and content records the package with
/// `identity` keeps, the data the user chooses to keep or delete.
fn saved_data(data: &ExtensionData, identity: &PackageIdentity) -> String {
    let kept = data
        .kept_now(&[DataKind::Settings, DataKind::Content])
        .describe(identity);
    format!("Saved data: {}", kept.unwrap_or_else(|| "none".into()))
}

/// The outcome of an uninstall of the packages titled `titles` (the one
/// asked about first) that was recorded, given what could not be removed of
/// each (`problems`, in the same order): a success only if every file it was
/// to remove is gone. Every package was uninstalled either way; a problem
/// is named with the package it belongs to.
fn outcome(titles: &[String], saved: SavedData, problems: &[Vec<String>]) -> Status {
    let (names, their) = match titles {
        [title] => (title.clone(), "its"),
        [title, dependents @ ..] => (dependents::with_dependents(title, dependents), "their"),
        [] => return Status::Idle,
    };
    if titles.len() == 1 {
        if !problems[0].is_empty() {
            return Status::Error(format!(
                "Uninstalled {names}, but {}.",
                problems[0].join("; ")
            ));
        }
    } else {
        let named: Vec<String> = titles
            .iter()
            .zip(problems)
            .filter(|(_, found)| !found.is_empty())
            .map(|(title, found)| format!("for {title}, {}", found.join("; ")))
            .collect();
        if !named.is_empty() {
            return Status::Error(format!(
                "Uninstalled {names}, but {}.",
                named.join("; and ")
            ));
        }
    }
    Status::Result(match (saved, titles.len()) {
        (SavedData::Keep, _) => {
            format!("Uninstalled {names}; {their} settings and content are kept")
        }
        (SavedData::Delete, 1) => format!("Uninstalled {names} and deleted its saved data"),
        (SavedData::Delete, _) => format!("Uninstalled {names}, and deleted their saved data"),
    })
}
