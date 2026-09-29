//! Continuing services (#48): a command whose manifest entry sets
//! `"service": true` has a service Pane keeps running in the background once
//! the user starts it in Manage extensions. Installing a package starts
//! nothing.
//!
//! The choice is Pane's own record, kept in `services.json` beside
//! `installed.json` by command id, as the other per-command choices are (see
//! `choices`), so it holds across restarts, disables, reloads and updates;
//! uninstalling the package forgets it. The service's status and how it last
//! ended are kept in memory only.
//!
//! **When a service runs.** Starting it (the user's choice) starts it at
//! once. After that it runs whenever its package's code may run: enabled,
//! not paused, not being reloaded, updated or uninstalled, the command
//! available on this system, and Pane running. So Pane starts it when Pane
//! starts and whenever its package's code starts again (enabled, reloaded,
//! updated or retried): a new generation starts it at once, whatever ended
//! the one before. One run at a time.
//!
//! **What a run's end does.** A service normally runs until its generation
//! ends or the user stops it; both stop it where it waits, discard what it
//! would still have answered, and drop its status. If it returns on its
//! own, an answer is shown as how it finished and it stays stopped until its
//! package's code starts again; an error is shown as its failure (an
//! ordinary outcome, never counted against the package), and so is a run
//! lost to a crash of the runtime; a crash (a trap) counts towards pausing
//! the package as any crash does (three within five minutes; see
//! `pausing`). After an error or a crash Pane starts it again
//! [`RESTART_DELAY`] after it ended, unless that paused the package, whose
//! service then waits for Retry.

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde_json::{Map, Value};

use super::background::{Finished, Pending};
use super::choices::{Choices, Record};
use super::{Entry, Launcher, Row, State, Status, Unavailable, off_thread};
use crate::extension_data::PackageData;
use crate::generation::End;
use crate::packages::{InstalledPackage, PackageIdentity, paused_reason};
use crate::runtime::{CallError, StatusSink, StopCall};

/// How long after an error or a crash Pane starts a service again.
pub const RESTART_DELAY: Duration = Duration::from_secs(60);

/// The services the user started, by command id, recorded in
/// `services.json` as `{ "version": 1, "services": ["<command id>"] }`.
pub(super) type ServiceChoices = BTreeSet<String>;

impl Choices for ServiceChoices {
    const FILE: &'static str = "services.json";
    const VERSION: u64 = 1;
    const WHAT: &'static str = "services";

    /// An entry that is not a string is left out.
    fn read(fields: &Map<String, Value>) -> Result<Self, String> {
        let services = fields
            .get("services")
            .and_then(Value::as_array)
            .ok_or("it has no `services`")?;
        Ok(services
            .iter()
            .filter_map(|command| Some(command.as_str()?.to_owned()))
            .collect())
    }

    fn write(&self) -> Map<String, Value> {
        let services = self.iter().cloned().map(Value::from).collect();
        Map::from_iter([("services".to_string(), Value::Array(services))])
    }

    fn restore(&mut self, command: &str, other: &Self) {
        if other.contains(command) {
            self.insert(command.to_owned());
        } else {
            self.remove(command);
        }
    }

    fn retain(&mut self, keep: &dyn Fn(&str) -> bool) -> bool {
        let before = self.len();
        BTreeSet::retain(self, |command| keep(command));
        self.len() != before
    }

    fn of(state: &mut State) -> &mut Record<Self> {
        &mut state.services.record
    }
}

/// How a service's last run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceOutcome {
    /// It finished on its own, answering this text.
    Finished(String),
    /// It answered this error, or was lost to a crash of the runtime.
    Failed(String),
    /// It crashed (trapped), with this reason.
    Crashed(String),
    /// Pane stopped it, for this reason: its package's code stopped.
    Stopped(String),
}

/// A continuing service, as [`Launcher::services`] lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceState {
    /// Its command's id (see [`crate::CommandRegistration::id`]).
    pub command: String,
    /// Its command's title.
    pub title: String,
    /// Whether the user started it (and has not stopped it).
    pub on: bool,
    /// Whether it is running.
    pub running: bool,
    /// The status it set last in its current run, if it is running and set
    /// one.
    pub status: Option<String>,
    /// How its last run ended, if it is on and a run ended since it was
    /// started or Pane started.
    pub last: Option<ServiceOutcome>,
    /// When Pane starts it again, if it ended with an error or a crash.
    pub restart_at: Option<SystemTime>,
}

/// The services' choices and runs.
#[derive(Default)]
pub(super) struct Services {
    /// The user's choices, and their record.
    pub(super) record: Record<ServiceChoices>,
    /// The runs in progress, by command id.
    running: HashMap<String, Run>,
    /// How the last run of each service ended, by command id.
    ended: HashMap<String, Ending>,
    /// Numbers the runs, to tell a run's end or status from a later run's.
    started: u64,
}

impl Services {
    pub(super) fn open(dir: &std::path::Path) -> Services {
        Services {
            record: Record::open(dir),
            ..Services::default()
        }
    }
}

/// A run in progress.
struct Run {
    number: u64,
    /// The status it set last.
    status: Option<String>,
    /// Stops the run once dropped: when the user stops it.
    _stop: StopCall,
}

/// How a service's last run ended, and what starts it again.
struct Ending {
    outcome: ServiceOutcome,
    /// Its package's code as the run had it: once that generation ended,
    /// the package's new code starts the service at once.
    data: PackageData,
    /// When Pane starts it again in the same generation, if it does.
    restart_at: Option<SystemTime>,
}

/// A run that ended, as the driver saw it.
pub(super) struct Ended {
    command: String,
    number: u64,
    data: PackageData,
    answer: Result<String, CallError>,
}

/// A command of an installed package with a service.
struct Declared<'a> {
    package: &'a InstalledPackage,
    command: crate::launcher::CommandRegistration,
    /// Why it does not run on this system, if it does not.
    unavailable: Option<String>,
}

/// The commands of `packages` with a service.
fn declared(packages: &[InstalledPackage]) -> Vec<Declared<'_>> {
    let mut found = Vec::new();
    for package in packages {
        let Ok(manifest) = &package.manifest else {
            continue;
        };
        let commands = package.available_commands().into_iter();
        for ((command, unavailable), declared) in commands.zip(&manifest.commands) {
            if declared.service {
                found.push(Declared {
                    package,
                    command,
                    unavailable,
                });
            }
        }
    }
    found
}

/// Why a run whose generation ended for `end` was stopped.
fn stopped_for(end: End, title: &str) -> String {
    match end {
        End::Disabled => format!("{title} was disabled"),
        End::Replaced => format!("{title} was reloaded or updated"),
        End::Uninstalled => format!("{title} was uninstalled"),
        End::Paused => format!("{title} was paused"),
    }
}

impl Launcher {
    /// The continuing services of the installed packages, enabled or not,
    /// with whether each was started, is running and what it shows.
    pub fn services(&self) -> Vec<ServiceState> {
        let state = self.lock();
        declared(&state.packages)
            .into_iter()
            .map(|declared| {
                let id = &declared.command.id;
                let on = state.services.record.chosen.contains(id);
                let run = state.services.running.get(id);
                let ended = state.services.ended.get(id).filter(|_| on);
                ServiceState {
                    command: id.clone(),
                    title: declared.command.title.clone(),
                    on,
                    running: run.is_some(),
                    status: run.and_then(|run| run.status.clone()),
                    last: ended.map(|ended| ended.outcome.clone()),
                    restart_at: ended.and_then(|ended| ended.restart_at),
                }
            })
            .collect()
    }

    /// Starts or stops the continuing service of the installed command with
    /// id `command`. Started, it runs at once if its package's code may run,
    /// and from then on whenever it may; stopped, a run in progress stops
    /// where it waits and none starts again. Await the returned future to
    /// record the choice; if it cannot be recorded, it goes back to what was
    /// last recorded. A command without a service, or a launcher that runs
    /// no background work, explains why instead.
    pub fn set_service(
        &self,
        command: &str,
        on: bool,
    ) -> impl Future<Output = ()> + Send + 'static {
        let mut state = self.lock();
        let change = self.switch_service(&mut state, command, on);
        drop(state);
        let launcher = self.clone();
        async move {
            if let Some(change) = change {
                launcher.finish_service_switch(change).await;
            }
        }
    }

    /// Starts the service of `command` if it is stopped, else stops it.
    pub(super) fn toggle_service(&self, state: &mut State, command: &str) -> Option<ServiceSwitch> {
        let on = !state.services.record.chosen.contains(command);
        // The list first, as it will be: the change belongs to its screen.
        let change = self.switch_service(state, command, on)?;
        self.show_extensions_at(
            state,
            |entry| matches!(entry, Entry::ToggleService(shown) if shown == command),
        );
        state.view.status = Status::Running;
        Some(ServiceSwitch {
            epoch: state.screen_epoch,
            ..change
        })
    }

    /// Starts or stops the service of `command` in Pane, explaining why it
    /// cannot; the change to record.
    fn switch_service(&self, state: &mut State, command: &str, on: bool) -> Option<ServiceSwitch> {
        let Some(declared) = declared(&state.packages)
            .into_iter()
            .find(|declared| declared.command.id == command)
        else {
            state.view.status = Status::Error(format!("{command} has no service"));
            return None;
        };
        let title = declared.command.title.clone();
        if on && self.background.is_none() {
            state.view.status = Status::Error(format!(
                "Cannot start {title}: this Pane runs no background work"
            ));
            return None;
        }
        let services = &mut state.services;
        let done = if on {
            if !services.record.chosen.insert(command.to_owned()) {
                return None;
            }
            services.ended.remove(command);
            format!("{title} runs in the background from now on")
        } else {
            if !services.record.chosen.remove(command) {
                return None;
            }
            // A run in progress stops where it waits; what it shows goes.
            services.running.remove(command);
            services.ended.remove(command);
            format!("{title} was stopped")
        };
        state.view.status = Status::Running;
        self.background_changed();
        Some(ServiceSwitch {
            command: command.to_owned(),
            done,
            epoch: state.screen_epoch,
        })
    }

    /// Records a service started or stopped, restoring what was last
    /// recorded if it cannot.
    pub(super) async fn finish_service_switch(&self, change: ServiceSwitch) {
        let ServiceSwitch {
            command,
            done,
            epoch,
        } = change;
        let launcher = self.clone();
        let saved = off_thread(move || launcher.save::<ServiceChoices>(Some(&command))).await;
        let mut state = self.lock();
        let status = match saved {
            Ok(()) => Status::Result(done),
            Err(problem) => {
                self.refresh(&mut state);
                self.background_changed();
                Status::Error(format!("Could not keep the service's state: {problem}"))
            }
        };
        if state.screen_epoch == epoch {
            state.view.status = status;
        }
    }

    /// Starts each service that is on, may run and is not running, unless
    /// it waits to be started again; returns what to wait for, when the
    /// next waiting one starts, and whether anything started.
    pub(super) fn start_due_services(
        &self,
        state: &mut State,
        now: SystemTime,
    ) -> (Vec<Pending>, Option<SystemTime>, bool) {
        let (Ok(runtime), Some(installation)) = (self.runtime(), &self.installation) else {
            return (Vec::new(), None, false);
        };
        let mut next: Option<SystemTime> = None;
        let mut startable = Vec::new();
        for declared in declared(&state.packages) {
            let id = &declared.command.id;
            let may_run = state.runs(declared.package)
                && declared.unavailable.is_none()
                && !state.changing.contains_key(&declared.package.identity);
            if !state.services.record.chosen.contains(id)
                || !may_run
                || state.services.running.contains_key(id)
            {
                continue;
            }
            if let Some(ended) = state.services.ended.get(id)
                && ended.data.stopped().is_none()
            {
                // Its generation goes on: only a restart after a failure
                // starts it again in it.
                match ended.restart_at {
                    Some(at) if at > now => {
                        next = Some(next.map_or(at, |next| next.min(at)));
                        continue;
                    }
                    Some(_) => {}
                    None => continue,
                }
            }
            startable.push((
                id.clone(),
                declared.command.component.clone(),
                declared.command.manifest_id().to_owned(),
                declared.package.identity.clone(),
            ));
        }
        let started_any = !startable.is_empty();
        let mut started: Vec<Pending> = Vec::new();
        for (id, component, command, identity) in startable {
            // Its package's code as of now: a disable or reload stops it.
            let data = installation.data.owned_by(&identity);
            state.services.started += 1;
            let number = state.services.started;
            let status = self.status_sink(&id, number);
            let (stop, answer) =
                runtime.run_service_with(&component, &command, data.clone(), status);
            state.services.running.insert(
                id.clone(),
                Run {
                    number,
                    status: None,
                    _stop: stop,
                },
            );
            state.services.ended.remove(&id);
            started.push(Box::pin(async move {
                Finished::Service(Ended {
                    command: id,
                    number,
                    data,
                    answer: answer.await,
                })
            }));
        }
        if started_any {
            self.refresh(state);
        }
        (started, next, started_any)
    }

    /// Where the status of the run numbered `number` of the service of
    /// `command` goes: shown while that run is the service's, else dropped.
    fn status_sink(&self, command: &str, number: u64) -> StatusSink {
        let launcher = self.downgrade();
        let command = command.to_owned();
        Arc::new(move |text| {
            if let Some(launcher) = launcher.upgrade() {
                launcher.show_service_status(&command, number, text);
            }
        })
    }

    /// Shows `text` as the status of the service of `command`, if its run
    /// numbered `number` is still running.
    fn show_service_status(&self, command: &str, number: u64, text: String) {
        {
            let mut state = self.lock();
            let Some(run) = state
                .services
                .running
                .get_mut(command)
                .filter(|run| run.number == number)
            else {
                return;
            };
            if run.status.as_deref() == Some(text.as_str()) {
                return;
            }
            run.status = Some(text);
            self.refresh(&mut state);
        }
        self.changed();
        if let Some(background) = &self.background {
            background.changed();
        }
    }

    /// Notes how a run ended, unless the service was stopped (or started
    /// again) meanwhile: an answer, error or crash becomes how it last
    /// ended, and an error or crash starts it again later; a run stopped
    /// with its package's code is shown as stopped, its answer discarded.
    pub(super) fn finish_service(&self, ended: Ended) {
        let Ended {
            command,
            number,
            data,
            answer,
        } = ended;
        let now = self.now();
        {
            let mut state = self.lock();
            let state = &mut *state;
            let current = state
                .services
                .running
                .get(&command)
                .is_some_and(|run| run.number == number);
            if !current {
                // Stopped by the user meanwhile: nothing of it is shown.
                return;
            }
            state.services.running.remove(&command);
            let package = super::choices::split(&command).0.to_owned();
            let package_title = state
                .packages
                .iter()
                .find(|package_| package_.identity.key() == package)
                .map_or_else(
                    || self.command_title(state, &command),
                    InstalledPackage::title,
                );
            let outcome = match (data.stopped(), answer) {
                // A crash is reported before its answer, and may have
                // paused the package: it is still the run's end.
                (_, Err(CallError::Trap(error))) => ServiceOutcome::Crashed(error),
                // Its code stopped: whatever it answered is discarded.
                (Some(end), _) => ServiceOutcome::Stopped(stopped_for(end, &package_title)),
                (None, Ok(text)) => ServiceOutcome::Finished(text),
                (None, Err(CallError::Guest(error))) => ServiceOutcome::Failed(error),
                (None, Err(error)) => ServiceOutcome::Failed(error.to_string()),
            };
            let restart_at = match outcome {
                ServiceOutcome::Failed(_) | ServiceOutcome::Crashed(_) => Some(now + RESTART_DELAY),
                ServiceOutcome::Finished(_) | ServiceOutcome::Stopped(_) => None,
            };
            state.services.ended.insert(
                command,
                Ending {
                    outcome,
                    data,
                    restart_at,
                },
            );
            self.refresh(state);
        }
        self.changed();
        if let Some(background) = &self.background {
            background.changed();
            // What starts it again is timed from now.
            background.wake();
        }
    }

    /// Forgets the services of the uninstalled package with `identity` (its
    /// runs stopped with its generation). Returns what writes the record
    /// without them, to run off the window's thread; nothing to write if it
    /// had none.
    pub(super) fn forget_services_of(
        &self,
        state: &mut State,
        identity: &PackageIdentity,
    ) -> Option<impl FnOnce() -> Result<(), String> + Send + 'static> {
        let key = identity.key();
        let of_it = |command: &String| super::choices::split(command).0 == key;
        state.services.running.retain(|command, _| !of_it(command));
        state.services.ended.retain(|command, _| !of_it(command));
        if !state.services.record.forget(identity) {
            return None;
        }
        let launcher = self.clone();
        Some(move || launcher.save::<ServiceChoices>(None))
    }

    /// The service rows of the extension list: one per command with a
    /// service of each enabled package, saying whether it runs and what it
    /// shows; activating one starts or stops it.
    pub(super) fn service_rows(&self, state: &State) -> Vec<(Row, Entry)> {
        declared(&state.packages)
            .into_iter()
            .filter(|declared| declared.package.enabled)
            .map(|declared| {
                let id = &declared.command.id;
                let title = &declared.command.title;
                let services = &state.services;
                let package = declared.package;
                let status = if !services.record.chosen.contains(id) {
                    "Stopped · Start it to run in the background while Pane runs".to_owned()
                } else if state.paused.is_paused(&package.identity) {
                    format!("Not running: {}", paused_reason(&package.title()))
                } else if self.background.is_none() {
                    "Not running: this Pane runs no background work".into()
                } else if let Some(run) = services.running.get(id) {
                    match &run.status {
                        Some(status) => format!("Running · {status}"),
                        None => "Running".into(),
                    }
                } else {
                    let again = "starts again a minute after it ended";
                    match services.ended.get(id).map(|ended| &ended.outcome) {
                        Some(ServiceOutcome::Finished(text)) => {
                            format!("Finished: {text} · Stop and start it to run it again")
                        }
                        Some(ServiceOutcome::Failed(error)) => {
                            format!("Failed: {error} · It {again}")
                        }
                        Some(ServiceOutcome::Crashed(error)) => {
                            format!("Crashed: {error} · It {again}")
                        }
                        Some(ServiceOutcome::Stopped(why)) => format!("Stopped: {why}"),
                        None => "Starting".into(),
                    }
                };
                let unavailable = declared.unavailable.clone().map(Unavailable::OnThisSystem);
                let entry = match &unavailable {
                    Some(reason) => Entry::Unavailable(reason.reason().to_owned()),
                    None => Entry::ToggleService(id.clone()),
                };
                let row = Row {
                    id: format!("service:{id}"),
                    title: format!("Service: {title}"),
                    subtitle: Some(format!("{status} · {}", package.identity)),
                    unavailable,
                };
                (row, entry)
            })
            .collect()
    }
}

/// A service started or stopped that has taken effect and is being
/// recorded.
pub(super) struct ServiceSwitch {
    command: String,
    /// The outcome once recorded.
    done: String,
    epoch: u64,
}
