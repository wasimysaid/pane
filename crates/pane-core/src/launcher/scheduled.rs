//! Scheduled tasks (#47): a command whose manifest entry declares a
//! schedule (`"schedule": { "everyMinutes": 15 }`) has a task Pane runs in
//! the background that often, once the user turns its schedule on in Manage
//! extensions. Installing a package schedules nothing.
//!
//! The choice, with when each task last ran and what its last run answered,
//! is Pane's own record, kept in `schedules.json` beside `installed.json`
//! by command id, as the other per-command choices are (see `choices`), so
//! it holds across restarts, disables, reloads and updates; uninstalling
//! the package forgets it.
//!
//! **When a task runs.** Turning its schedule on runs it at once. After
//! that it is due one interval after its last run began. A run falls due
//! only while its package's code may run: enabled, not paused, not being
//! reloaded, updated or uninstalled, the command available on this system,
//! and Pane running. A task that fell due meanwhile (Pane was not running,
//! the package was disabled or paused) runs once as soon as it can again,
//! never once per interval missed; the interval then counts from that run.
//! A task runs no more than once at a time: a run still going when the next
//! falls due is not doubled, and the next is due one interval after it
//! began. A last run recorded in the future (the system's time went back)
//! counts as due.
//!
//! **What a run's end does.** An answer is shown as the task's latest
//! result, an error as its latest failure; either way the schedule goes on,
//! and an error never counts against the package. A crash (a trap) counts
//! towards pausing the package as any crash does (three within five minutes;
//! see `pausing`), and a paused package's schedule waits for Retry. A run
//! stopped because its package's generation ended (it was disabled,
//! reloaded, updated, paused or uninstalled) or its schedule was turned off
//! is stopped where it waits, and its answer, if it still comes, is
//! discarded: the latest result stays the one before, and the stop is shown
//! until the next run. The new code of a reloaded or updated package is not
//! run at once for the run it stopped: its next run is due one interval
//! after the stopped one began.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::time::{Duration, SystemTime};

use serde_json::{Map, Value};

use super::background::{Finished, Pending};
use super::choices::{Choices, Record};
use super::{Entry, Launcher, Row, State, Status, Unavailable, off_thread};
use crate::extension_data::PackageData;
use crate::generation::End;
use crate::packages::{InstalledPackage, PackageIdentity, Schedule, paused_reason};
use crate::runtime::{CallError, StopCall};

/// A task the user turned on, as recorded: when it last ran, and how its
/// last run that was not stopped ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct TaskRecord {
    last_run: Option<SystemTime>,
    last: Option<TaskOutcome>,
}

/// The scheduled tasks the user turned on, by command id, recorded in
/// `schedules.json` as `{ "version": 1, "tasks": { "<command id>":
/// { "lastRun": <seconds since 1970>, "outcome": "answered", "text": "…" } } }`.
pub(super) type TaskChoices = BTreeMap<String, TaskRecord>;

impl Choices for TaskChoices {
    const FILE: &'static str = "schedules.json";
    const VERSION: u64 = 1;
    const WHAT: &'static str = "schedules";

    /// An entry that is not an object is left out; a field it cannot use is
    /// ignored.
    fn read(fields: &Map<String, Value>) -> Result<Self, String> {
        let tasks = fields
            .get("tasks")
            .and_then(Value::as_object)
            .ok_or("it has no `tasks`")?;
        Ok(tasks
            .iter()
            .filter_map(|(command, task)| {
                let task = task.as_object()?;
                let last_run = task
                    .get("lastRun")
                    .and_then(Value::as_u64)
                    .map(|seconds| SystemTime::UNIX_EPOCH + Duration::from_secs(seconds));
                let text = task.get("text").and_then(Value::as_str).map(str::to_owned);
                let last = match (task.get("outcome").and_then(Value::as_str), text) {
                    (Some("answered"), Some(text)) => Some(TaskOutcome::Answered(text)),
                    (Some("failed"), Some(text)) => Some(TaskOutcome::Failed(text)),
                    (Some("crashed"), Some(text)) => Some(TaskOutcome::Crashed(text)),
                    _ => None,
                };
                Some((command.clone(), TaskRecord { last_run, last }))
            })
            .collect())
    }

    fn write(&self) -> Map<String, Value> {
        let tasks = self
            .iter()
            .map(|(command, task)| {
                let mut fields = Map::new();
                if let Some(seconds) = task
                    .last_run
                    .and_then(|at| at.duration_since(SystemTime::UNIX_EPOCH).ok())
                {
                    fields.insert("lastRun".into(), seconds.as_secs().into());
                }
                let last = match &task.last {
                    Some(TaskOutcome::Answered(text)) => Some(("answered", text)),
                    Some(TaskOutcome::Failed(text)) => Some(("failed", text)),
                    Some(TaskOutcome::Crashed(text)) => Some(("crashed", text)),
                    Some(TaskOutcome::Stopped(_)) | None => None,
                };
                if let Some((outcome, text)) = last {
                    fields.insert("outcome".into(), outcome.into());
                    fields.insert("text".into(), text.clone().into());
                }
                (command.clone(), Value::Object(fields))
            })
            .collect();
        Map::from_iter([("tasks".to_string(), Value::Object(tasks))])
    }

    fn restore(&mut self, command: &str, other: &Self) {
        match other.get(command) {
            Some(task) => self.insert(command.to_owned(), task.clone()),
            None => self.remove(command),
        };
    }

    fn retain(&mut self, keep: &dyn Fn(&str) -> bool) -> bool {
        let before = self.len();
        BTreeMap::retain(self, |command, _| keep(command));
        self.len() != before
    }

    fn of(state: &mut State) -> &mut Record<Self> {
        &mut state.tasks.record
    }
}

/// How a scheduled task's run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskOutcome {
    /// It answered this text.
    Answered(String),
    /// It answered this error: an ordinary outcome.
    Failed(String),
    /// It crashed (trapped), with this reason.
    Crashed(String),
    /// Pane stopped it, for this reason, before it answered: its package's
    /// code stopped, or its schedule was turned off. Not recorded.
    Stopped(String),
}

/// A scheduled task, as [`Launcher::scheduled_tasks`] lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduledTask {
    /// Its command's id (see [`crate::CommandRegistration::id`]).
    pub command: String,
    /// Its command's title.
    pub title: String,
    pub schedule: Schedule,
    /// Whether the user turned its schedule on.
    pub on: bool,
    /// Whether a run of it is in progress.
    pub running: bool,
    /// When its last run began, if it is on and has run.
    pub last_run: Option<SystemTime>,
    /// How its last run ended, if it is on and a run has ended since.
    pub last: Option<TaskOutcome>,
}

/// The scheduled tasks' choices and runs.
#[derive(Default)]
pub(super) struct Tasks {
    /// The user's choices and the last runs, and their record.
    pub(super) record: Record<TaskChoices>,
    /// The runs in progress, by command id.
    running: HashMap<String, Run>,
    /// The last run of a task that was stopped, by command id, with why:
    /// shown until its next run ends.
    stopped: HashMap<String, String>,
    /// Numbers the runs, to tell a run's end from a later run's.
    started: u64,
}

impl Tasks {
    pub(super) fn open(dir: &std::path::Path) -> Tasks {
        Tasks {
            record: Record::open(dir),
            ..Tasks::default()
        }
    }
}

/// A run in progress.
struct Run {
    number: u64,
    /// Stops the run once dropped: when the schedule is turned off.
    _stop: StopCall,
}

/// A run that ended, as the driver saw it.
pub(super) struct Ended {
    command: String,
    number: u64,
    data: PackageData,
    answer: Result<String, CallError>,
}

/// A scheduled command of an installed package.
struct Scheduled<'a> {
    package: &'a InstalledPackage,
    command: crate::launcher::CommandRegistration,
    schedule: Schedule,
    /// Why it does not run on this system, if it does not.
    unavailable: Option<String>,
}

/// The scheduled commands of `packages`.
fn scheduled(packages: &[InstalledPackage]) -> Vec<Scheduled<'_>> {
    let mut found = Vec::new();
    for package in packages {
        let Ok(manifest) = &package.manifest else {
            continue;
        };
        let commands = package.available_commands().into_iter();
        for ((command, unavailable), declared) in commands.zip(&manifest.commands) {
            if let Some(schedule) = declared.schedule {
                found.push(Scheduled {
                    package,
                    command,
                    schedule,
                    unavailable,
                });
            }
        }
    }
    found
}

/// When a task with `record` and `schedule` is due, at `now`.
fn due(record: &TaskRecord, schedule: Schedule, now: SystemTime) -> SystemTime {
    match record.last_run {
        Some(last) if last <= now => last + schedule.every(),
        // Never run, or recorded in the future: due now.
        _ => now,
    }
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
    /// The scheduled tasks of the installed packages, enabled or not, with
    /// whether each is on and how its last run ended.
    pub fn scheduled_tasks(&self) -> Vec<ScheduledTask> {
        let state = self.lock();
        scheduled(&state.packages)
            .into_iter()
            .map(|scheduled| {
                let id = &scheduled.command.id;
                let record = state.tasks.record.chosen.get(id);
                let last = match state.tasks.stopped.get(id) {
                    Some(why) => Some(TaskOutcome::Stopped(why.clone())),
                    None => record.and_then(|record| record.last.clone()),
                };
                ScheduledTask {
                    command: id.clone(),
                    title: scheduled.command.title.clone(),
                    schedule: scheduled.schedule,
                    on: record.is_some(),
                    running: state.tasks.running.contains_key(id),
                    last_run: record.and_then(|record| record.last_run),
                    last: last.filter(|_| record.is_some()),
                }
            })
            .collect()
    }

    /// Turns the schedule of the scheduled task of the installed command
    /// with id `command` on or off. On, it runs at once, then as its
    /// schedule says; off, a run in progress stops where it waits and no
    /// other starts. Await the returned future to record the choice; if it
    /// cannot be recorded, it goes back to what was last recorded. A command
    /// without a schedule, or a launcher that runs no background work,
    /// explains why instead.
    pub fn set_task_schedule(
        &self,
        command: &str,
        on: bool,
    ) -> impl Future<Output = ()> + Send + 'static {
        let mut state = self.lock();
        let change = self.switch_task(&mut state, command, on);
        drop(state);
        let launcher = self.clone();
        async move {
            if let Some(change) = change {
                launcher.finish_task_switch(change).await;
            }
        }
    }

    /// Turns the schedule of `command` on if it is off, else off.
    pub(super) fn toggle_task(&self, state: &mut State, command: &str) -> Option<TaskSwitch> {
        let on = !state.tasks.record.chosen.contains_key(command);
        // The list first, as it will be: the change belongs to its screen.
        let change = self.switch_task(state, command, on)?;
        self.show_extensions_at(
            state,
            |entry| matches!(entry, Entry::ToggleTask(shown) if shown == command),
        );
        state.view.status = Status::Running;
        Some(TaskSwitch {
            epoch: state.screen_epoch,
            ..change
        })
    }

    /// Turns the schedule of `command` on or off in Pane, explaining why it
    /// cannot; the change to record.
    fn switch_task(&self, state: &mut State, command: &str, on: bool) -> Option<TaskSwitch> {
        let Some(scheduled) = scheduled(&state.packages)
            .into_iter()
            .find(|scheduled| scheduled.command.id == command)
        else {
            state.view.status = Status::Error(format!("{command} has no schedule"));
            return None;
        };
        let title = scheduled.command.title.clone();
        let schedule = scheduled.schedule;
        if on && self.background.is_none() {
            state.view.status = Status::Error(format!(
                "Cannot run {title} on a schedule: this Pane runs no background work"
            ));
            return None;
        }
        let done = if on {
            if state.tasks.record.chosen.contains_key(command) {
                return None;
            }
            state
                .tasks
                .record
                .chosen
                .insert(command.to_owned(), TaskRecord::default());
            format!("{title} runs {schedule} in the background from now on")
        } else {
            state.tasks.record.chosen.remove(command)?;
            // A run in progress stops where it waits.
            state.tasks.running.remove(command);
            state.tasks.stopped.remove(command);
            format!("{title} no longer runs on a schedule")
        };
        state.view.status = Status::Running;
        self.background_changed();
        Some(TaskSwitch {
            command: command.to_owned(),
            done,
            epoch: state.screen_epoch,
        })
    }

    /// Records a schedule turned on or off, restoring what was last
    /// recorded if it cannot.
    pub(super) async fn finish_task_switch(&self, change: TaskSwitch) {
        let TaskSwitch {
            command,
            done,
            epoch,
        } = change;
        let launcher = self.clone();
        let saved = off_thread(move || launcher.save::<TaskChoices>(Some(&command))).await;
        let mut state = self.lock();
        let status = match saved {
            Ok(()) => Status::Result(done),
            Err(problem) => {
                self.refresh(&mut state);
                self.background_changed();
                Status::Error(format!("Could not keep the schedule: {problem}"))
            }
        };
        if state.screen_epoch == epoch {
            state.view.status = status;
        }
    }

    /// Starts each task that is due at `now` and may run; returns what to
    /// wait for, when the next falls due, and whether the record changed.
    pub(super) fn start_due_tasks(
        &self,
        state: &mut State,
        now: SystemTime,
    ) -> (Vec<Pending>, Option<SystemTime>, bool) {
        let (Ok(runtime), Some(installation)) = (self.runtime(), &self.installation) else {
            return (Vec::new(), None, false);
        };
        let mut started: Vec<Pending> = Vec::new();
        let mut next: Option<SystemTime> = None;
        let mut runnable = Vec::new();
        for scheduled in scheduled(&state.packages) {
            let id = &scheduled.command.id;
            let may_run = state.runs(scheduled.package)
                && scheduled.unavailable.is_none()
                && !state.changing.contains_key(&scheduled.package.identity);
            let Some(record) = state.tasks.record.chosen.get(id) else {
                continue;
            };
            if !may_run || state.tasks.running.contains_key(id) {
                continue;
            }
            let at = due(record, scheduled.schedule, now);
            if at > now {
                next = Some(next.map_or(at, |next| next.min(at)));
                continue;
            }
            runnable.push((
                id.clone(),
                scheduled.command.component.clone(),
                scheduled.command.manifest_id().to_owned(),
                scheduled.package.identity.clone(),
                scheduled.schedule,
            ));
        }
        let record = !runnable.is_empty();
        for (id, component, command, identity, schedule) in runnable {
            // Its package's code as of now: a disable or reload stops it.
            let data = installation.data.owned_by(&identity);
            let (stop, answer) = runtime.run_task_with(&component, &command, data.clone());
            state.tasks.started += 1;
            let number = state.tasks.started;
            state.tasks.running.insert(
                id.clone(),
                Run {
                    number,
                    _stop: stop,
                },
            );
            if let Some(record) = state.tasks.record.chosen.get_mut(&id) {
                record.last_run = Some(now);
            }
            let at = now + schedule.every();
            next = Some(next.map_or(at, |next| next.min(at)));
            started.push(Box::pin(async move {
                Finished::Task(Ended {
                    command: id,
                    number,
                    data,
                    answer: answer.await,
                })
            }));
        }
        if record {
            self.refresh(state);
        }
        (started, next, record)
    }

    /// Notes how a run ended, unless its schedule was turned off (or on
    /// again) meanwhile: its answer, error or crash becomes the task's
    /// latest result, recorded; a run stopped with its package's code is
    /// shown as stopped, its answer discarded.
    pub(super) fn finish_task(&self, ended: Ended) {
        let Ended {
            command,
            number,
            data,
            answer,
        } = ended;
        {
            let mut state = self.lock();
            let state = &mut *state;
            let current = state
                .tasks
                .running
                .get(&command)
                .is_some_and(|run| run.number == number);
            if !current {
                // Turned off meanwhile: nothing of it is shown.
                return;
            }
            state.tasks.running.remove(&command);
            let title = self.command_title(state, &command);
            let package = super::choices::split(&command).0.to_owned();
            let package_title = state
                .packages
                .iter()
                .find(|package_| package_.identity.key() == package)
                .map_or_else(|| title.clone(), InstalledPackage::title);
            let outcome = match (data.stopped(), answer) {
                // A crash is reported before its answer, and may have
                // paused the package: it is still the run's end.
                (_, Err(CallError::Trap(error))) => TaskOutcome::Crashed(error),
                // Its code stopped: whatever it answered is discarded.
                (Some(end), _) => TaskOutcome::Stopped(stopped_for(end, &package_title)),
                (None, Ok(text)) => TaskOutcome::Answered(text),
                (None, Err(CallError::Guest(error))) => TaskOutcome::Failed(error),
                (None, Err(CallError::Cancelled)) => {
                    TaskOutcome::Stopped("its schedule was turned off".into())
                }
                (None, Err(error)) => TaskOutcome::Failed(error.to_string()),
            };
            match outcome {
                TaskOutcome::Stopped(why) => {
                    state.tasks.stopped.insert(command.clone(), why);
                }
                outcome => {
                    state.tasks.stopped.remove(&command);
                    if let Some(record) = state.tasks.record.chosen.get_mut(&command) {
                        record.last = Some(outcome);
                    }
                }
            }
            self.refresh(state);
        }
        // Recorded before anyone is told, so what they read is on record.
        self.record_tasks();
        self.changed();
        if let Some(background) = &self.background {
            background.changed();
        }
    }

    /// Writes the schedules' record as it is now; a failure is reported on
    /// standard error, and the next change writes it again.
    pub(super) fn record_tasks(&self) {
        if let Err(problem) = self.save::<TaskChoices>(None) {
            eprintln!("pane: could not record the schedules' last runs: {problem}");
        }
    }

    /// Forgets the schedules of the uninstalled package with `identity`
    /// (its runs stopped with its generation). Returns what writes the
    /// record without them, to run off the window's thread; nothing to write
    /// if it had none.
    pub(super) fn forget_tasks_of(
        &self,
        state: &mut State,
        identity: &PackageIdentity,
    ) -> Option<impl FnOnce() -> Result<(), String> + Send + 'static> {
        let key = identity.key();
        let of_it = |command: &String| super::choices::split(command).0 == key;
        state.tasks.running.retain(|command, _| !of_it(command));
        state.tasks.stopped.retain(|command, _| !of_it(command));
        if !state.tasks.record.forget(identity) {
            return None;
        }
        let launcher = self.clone();
        Some(move || launcher.save::<TaskChoices>(None))
    }

    /// The schedule rows of the extension list: one per scheduled command
    /// of each enabled package, saying whether its schedule is on and how
    /// its last run ended; activating one turns it on or off.
    pub(super) fn schedule_rows(&self, state: &State) -> Vec<(Row, Entry)> {
        scheduled(&state.packages)
            .into_iter()
            .filter(|scheduled| scheduled.package.enabled)
            .map(|scheduled| {
                let id = &scheduled.command.id;
                let title = &scheduled.command.title;
                let schedule = scheduled.schedule;
                let tasks = &state.tasks;
                let status = match tasks.record.chosen.get(id) {
                    None => format!("Off · Run it in the background {schedule}"),
                    Some(record) => {
                        let package = scheduled.package;
                        let latest = if state.paused.is_paused(&package.identity) {
                            format!("Not running: {}", paused_reason(&package.title()))
                        } else if self.background.is_none() {
                            "Not running: this Pane runs no background work".into()
                        } else if tasks.running.contains_key(id) {
                            "Running now".into()
                        } else {
                            match (tasks.stopped.get(id), &record.last) {
                                (Some(why), _) => format!("Last run stopped: {why}"),
                                (None, Some(TaskOutcome::Answered(text))) => {
                                    format!("Last run: {text}")
                                }
                                (None, Some(TaskOutcome::Failed(error))) => {
                                    format!("Last run failed: {error}")
                                }
                                (None, Some(TaskOutcome::Crashed(error))) => {
                                    format!("Last run crashed: {error}")
                                }
                                (None, Some(TaskOutcome::Stopped(why))) => {
                                    format!("Last run stopped: {why}")
                                }
                                (None, None) => "Not run yet".into(),
                            }
                        };
                        let schedule = schedule.to_string();
                        let mut schedule = schedule.chars();
                        let capital: String = schedule
                            .next()
                            .map(|first| first.to_uppercase().chain(schedule).collect())
                            .unwrap_or_default();
                        format!("On · {capital} · {latest}")
                    }
                };
                let unavailable = scheduled.unavailable.clone().map(Unavailable::OnThisSystem);
                let entry = match &unavailable {
                    Some(reason) => Entry::Unavailable(reason.reason().to_owned()),
                    None => Entry::ToggleTask(id.clone()),
                };
                let row = Row {
                    id: format!("schedule:{id}"),
                    title: format!("Schedule: {title}"),
                    subtitle: Some(format!("{status} · {}", scheduled.package.identity)),
                    unavailable,
                };
                (row, entry)
            })
            .collect()
    }
}

/// A schedule turned on or off that has taken effect and is being
/// recorded.
pub(super) struct TaskSwitch {
    command: String,
    /// The outcome once recorded.
    done: String,
    epoch: u64,
}
