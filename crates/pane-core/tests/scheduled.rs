//! Scheduled tasks (#47) through the launcher's public interface: the user
//! turns a command's schedule on in Manage extensions, and Pane runs its
//! task in the background as the schedule says, on a clock the tests move
//! by hand ([`ManualClock`]), so no test waits for a schedule. The task is a
//! real guest: Ticks, the background sample from `cargo xtask guests`, in
//! Rust, JavaScript and TypeScript alike.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use futures::executor::block_on;
use pane_core::clock::ManualClock;
use pane_core::{
    Launcher, PackageIdentity, Runtime, SavedData, ScheduledTask, Screen, Status, TaskOutcome,
};
use serde_json::Value;
use tempfile::TempDir;

const MINUTE: Duration = Duration::from_secs(60);

struct Fixture {
    /// The assembled package under `target/guests/packages`.
    package: &'static str,
    title: &'static str,
}

const RUST: Fixture = Fixture {
    package: "sample-background",
    title: "Background sample",
};
const JAVASCRIPT: Fixture = Fixture {
    package: "sample-background-js",
    title: "JavaScript background sample",
};
const TYPESCRIPT: Fixture = Fixture {
    package: "sample-background-ts",
    title: "TypeScript background sample",
};
const ALL: [Fixture; 3] = [RUST, JAVASCRIPT, TYPESCRIPT];

/// When the tests' clock starts: a fixed time, whole seconds, as recorded.
fn start() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

struct Pane {
    sources: TempDir,
    data: TempDir,
    clock: ManualClock,
}

impl Pane {
    fn new() -> Pane {
        Pane {
            sources: tempfile::tempdir().unwrap(),
            data: tempfile::tempdir().unwrap(),
            clock: ManualClock::new(start()),
        }
    }

    fn packages_dir(&self) -> PathBuf {
        self.data.path().join("extensions")
    }

    /// A launcher on this data folder running background work on the
    /// tests' clock, with its own runtime; a new one is a restart of Pane.
    fn start(&self) -> (Launcher, Runtime) {
        let runtime = Runtime::start().unwrap();
        let launcher = Launcher::with_packages(Ok(runtime.clone()), vec![], self.packages_dir())
            .with_background(Arc::new(self.clock.clone()));
        (launcher, runtime)
    }

    /// Copies the assembled package of `fixture` to a source folder and
    /// installs it; its identity.
    fn install(&self, launcher: &Launcher, fixture: &Fixture) -> PackageIdentity {
        let assembled = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/guests/packages")
            .join(fixture.package);
        assert!(
            assembled.exists(),
            "{} is missing; run `cargo xtask guests`",
            assembled.display()
        );
        let folder = self.sources.path().join(fixture.package);
        fs::create_dir_all(&folder).unwrap();
        for entry in fs::read_dir(&assembled).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), folder.join(entry.file_name())).unwrap();
        }
        block_on(launcher.install_package(&folder));
        assert!(
            matches!(launcher.view().status, Status::Result(_)),
            "{:?}",
            launcher.view().status
        );
        launcher
            .packages()
            .into_iter()
            .find(|package| package.title() == fixture.title)
            .expect("installed")
            .identity
    }

    /// The value the package keeps under `key` in its content, if any.
    fn content(&self, key: &str) -> Option<String> {
        let text = fs::read_to_string(self.packages_dir().join("content.json")).ok()?;
        let content: Value = serde_json::from_str(&text).unwrap();
        content["packages"]
            .as_object()?
            .values()
            .find_map(|values| Some(values.get(key)?.as_str()?.to_owned()))
    }

    /// The record of schedules, as written.
    fn record(&self) -> Value {
        let text = fs::read_to_string(self.packages_dir().join("schedules.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }
}

/// The one scheduled task, Ticks.
fn ticks(launcher: &Launcher) -> ScheduledTask {
    let tasks = launcher.scheduled_tasks();
    assert_eq!(tasks.len(), 1, "{tasks:?}");
    tasks.into_iter().next().unwrap()
}

/// Waits, on the launcher's background changes, until `done` holds.
fn until(launcher: &Launcher, what: &str, done: impl Fn(&ScheduledTask) -> bool) {
    let mut changes = launcher.background_changes();
    let waiting = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done(&ticks(launcher)) {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "{what} did not happen: {:?}",
            ticks(launcher)
        );
        let _ = waiting.block_on(async { tokio::time::timeout(left, changes.next()).await });
    }
}

/// Waits until the task's run numbered `count` answered.
fn answered(launcher: &Launcher, count: u32) {
    let answer = format!("Ticked {count} times");
    until(launcher, &answer, |task| {
        !task.running && task.last == Some(TaskOutcome::Answered(answer.clone()))
    });
}

/// Waits, briefly, until `seen` holds; for what a guest saved.
fn until_seen(what: &str, seen: impl Fn() -> bool) {
    let started = Instant::now();
    while !seen() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{what} was not seen"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Opens Ticks and runs its item titled `title` (how the next runs behave).
fn choose(launcher: &Launcher, title: &str) {
    launcher.back();
    launcher.back();
    block_on(launcher.set_query("ticks"));
    block_on(launcher.activate_selected());
    let view = launcher.view();
    assert_eq!(view.screen, Screen::Command, "{view:?}");
    let index = view
        .rows
        .iter()
        .position(|row| row.title.starts_with(title))
        .unwrap_or_else(|| panic!("no item {title:?}: {:?}", view.rows));
    launcher.select(index);
    block_on(launcher.activate_selected());
    assert!(
        matches!(launcher.view().status, Status::Result(_)),
        "{:?}",
        launcher.view().status
    );
    launcher.back();
}

/// Shows Manage extensions, and the subtitle of its row titled `title`.
fn managed(launcher: &Launcher, title: &str) -> String {
    launcher.back();
    launcher.back();
    block_on(launcher.set_query("manage extensions"));
    block_on(launcher.activate_selected());
    let view = launcher.view();
    assert!(matches!(view.screen, Screen::Extensions { .. }), "{view:?}");
    view.rows
        .iter()
        .find(|row| row.title == title)
        .and_then(|row| row.subtitle.clone())
        .unwrap_or_else(|| panic!("no row {title:?}: {:?}", view.rows))
}

#[test]
fn installing_schedules_nothing_and_a_schedule_turned_on_runs_at_once_then_on_time() {
    for fixture in ALL {
        let pane = Pane::new();
        let (launcher, runtime) = pane.start();
        pane.install(&launcher, &fixture);

        let task = ticks(&launcher);
        assert_eq!(task.title, "Ticks");
        assert_eq!(task.schedule.every(), MINUTE);
        assert!(!task.on && !task.running && task.last_run.is_none());
        assert!(
            managed(&launcher, "Schedule: Ticks")
                .starts_with("Off · Run it in the background every minute"),
            "{}",
            managed(&launcher, "Schedule: Ticks")
        );
        pane.clock.advance(10 * MINUTE);
        assert_eq!(
            block_on(runtime.background_running()),
            Vec::<PathBuf>::new()
        );
        assert_eq!(pane.content("ticks"), None, "installing ran it");

        // Turned on in Manage extensions (Enter on its row): it runs at once.
        let row = launcher
            .view()
            .rows
            .iter()
            .position(|row| row.title == "Schedule: Ticks")
            .unwrap();
        launcher.select(row);
        block_on(launcher.activate_selected());
        assert_eq!(
            launcher.view().status,
            Status::Result("Ticks runs every minute in the background from now on".into())
        );
        answered(&launcher, 1);
        let first = start() + 10 * MINUTE;
        assert_eq!(ticks(&launcher).last_run, Some(first));
        assert_eq!(pane.content("ticks").as_deref(), Some("1"));
        let subtitle = managed(&launcher, "Schedule: Ticks");
        assert!(
            subtitle.starts_with("On · Every minute · Last run: Ticked 1 times"),
            "{subtitle}"
        );

        // Not before its time: a run at 59 seconds would have been the one
        // recorded, and nothing would be due at a minute.
        pane.clock.advance(MINUTE - Duration::from_secs(1));
        pane.clock.advance(Duration::from_secs(1));
        answered(&launcher, 2);
        assert_eq!(ticks(&launcher).last_run, Some(first + MINUTE));

        // Ten minutes at once are one run, not ten.
        pane.clock.advance(10 * MINUTE);
        answered(&launcher, 3);
        assert_eq!(ticks(&launcher).last_run, Some(first + 11 * MINUTE));
        pane.clock.advance(MINUTE);
        answered(&launcher, 4);

        // Turned off, nothing runs any more; the result goes with it.
        block_on(launcher.set_task_schedule(&task.command, false));
        let task = ticks(&launcher);
        assert!(!task.on && task.last.is_none(), "{task:?}");
        pane.clock.advance(10 * MINUTE);
        assert_eq!(pane.content("ticks").as_deref(), Some("4"));
        assert_eq!(
            block_on(runtime.background_running()),
            Vec::<PathBuf>::new()
        );
        let record = pane.record();
        assert_eq!(record["tasks"], serde_json::json!({}), "{record}");
    }
}

#[test]
fn a_schedule_holds_across_a_restart_and_what_fell_due_meanwhile_runs_once() {
    let pane = Pane::new();
    let command = {
        let (launcher, _runtime) = pane.start();
        pane.install(&launcher, &RUST);
        let command = ticks(&launcher).command;
        block_on(launcher.set_task_schedule(&command, true));
        answered(&launcher, 1);
        command
    };
    let record = pane.record();
    let recorded = &record["tasks"][&command];
    assert_eq!(recorded["lastRun"], 1_800_000_000u64, "{record}");
    assert_eq!(recorded["outcome"], "answered");
    assert_eq!(recorded["text"], "Ticked 1 times");

    // Restarted before it is due: nothing runs until it is.
    pane.clock.advance(Duration::from_secs(30));
    let (launcher, _runtime) = pane.start();
    let task = ticks(&launcher);
    assert!(task.on, "{task:?}");
    assert_eq!(
        task.last,
        Some(TaskOutcome::Answered("Ticked 1 times".into()))
    );
    pane.clock.advance(Duration::from_secs(30));
    answered(&launcher, 2);
    assert_eq!(ticks(&launcher).last_run, Some(start() + MINUTE));
    drop(launcher);

    // Pane was not running for an hour: one run at once, not sixty.
    pane.clock.advance(60 * MINUTE);
    let (launcher, _runtime) = pane.start();
    answered(&launcher, 3);
    assert_eq!(ticks(&launcher).last_run, Some(start() + 61 * MINUTE));
    assert_eq!(pane.content("ticks").as_deref(), Some("3"));

    // A last run recorded in the future (the time went back) is due now.
    drop(launcher);
    pane.clock.set(start());
    let (launcher, _runtime) = pane.start();
    answered(&launcher, 4);
    assert_eq!(ticks(&launcher).last_run, Some(start()));
}

#[test]
fn an_error_is_an_ordinary_outcome_and_repeated_crashes_pause_the_package() {
    for fixture in ALL {
        let pane = Pane::new();
        let (launcher, _runtime) = pane.start();
        let identity = pane.install(&launcher, &fixture);
        choose(&launcher, "Answer with an error");
        let command = ticks(&launcher).command;
        block_on(launcher.set_task_schedule(&command, true));
        let failed = |task: &ScheduledTask| {
            matches!(&task.last, Some(TaskOutcome::Failed(error))
                if error.contains("Ticks refuses to count, to show how a failed run looks"))
        };
        until(&launcher, "a failed run", |task| {
            !task.running && failed(task)
        });
        let subtitle = managed(&launcher, "Schedule: Ticks");
        assert!(subtitle.contains("Last run failed: "), "{subtitle}");
        // The schedule goes on, again and again, and never pauses it.
        for _ in 0..3 {
            let last_run = ticks(&launcher).last_run;
            pane.clock.advance(MINUTE);
            until(&launcher, "another failed run", |task| {
                !task.running && task.last_run != last_run && failed(task)
            });
        }
        let subtitle = managed(&launcher, fixture.title);
        assert!(subtitle.starts_with("Enabled · "), "{subtitle}");

        choose(&launcher, "Crash on purpose");
        for crash in 1..=3 {
            let last_run = ticks(&launcher).last_run;
            pane.clock.advance(MINUTE);
            until(&launcher, &format!("crash {crash}"), |task| {
                !task.running
                    && task.last_run != last_run
                    && matches!(task.last, Some(TaskOutcome::Crashed(_)))
            });
        }
        // Three crashes within five minutes pause it, as any crashes do.
        let subtitle = managed(&launcher, fixture.title);
        assert!(
            subtitle.starts_with("Enabled · Paused after crashing"),
            "{subtitle}"
        );
        let subtitle = managed(&launcher, "Schedule: Ticks");
        assert!(subtitle.contains("Not running: "), "{subtitle}");
        let last_run = ticks(&launcher).last_run;
        pane.clock.advance(10 * MINUTE);
        assert_eq!(ticks(&launcher).last_run, last_run, "a paused task ran");

        // Retry runs it again: what fell due runs once, at once.
        block_on(launcher.retry_start(&identity));
        until(&launcher, "a run after Retry", |task| {
            !task.running && task.last_run != last_run
        });
        assert_eq!(ticks(&launcher).last_run, Some(pane.clock_now()));
    }
}

#[test]
fn disabling_reloading_or_turning_off_stops_a_waiting_run_and_its_answer_is_discarded() {
    for fixture in ALL {
        let pane = Pane::new();
        let (launcher, runtime) = pane.start();
        let identity = pane.install(&launcher, &fixture);
        choose(&launcher, "Wait 10 seconds in each run");
        let command = ticks(&launcher).command;
        let waiting = || pane.content("tick-wait").as_deref() == Some("started");
        let stopped = |why: &str| {
            let why = why.to_owned();
            move |task: &ScheduledTask| {
                !task.running && task.last == Some(TaskOutcome::Stopped(why.clone()))
            }
        };

        block_on(launcher.set_task_schedule(&command, true));
        until_seen("the first run waiting", waiting);
        let asked = Instant::now();
        block_on(launcher.set_enabled(&identity, false));
        let title = fixture.title;
        until(
            &launcher,
            "the run stopped by the disable",
            stopped(&format!("{title} was disabled")),
        );
        assert!(
            asked.elapsed() < Duration::from_secs(8),
            "it was not stopped"
        );
        assert_eq!(
            block_on(runtime.background_running()),
            Vec::<PathBuf>::new()
        );
        assert_eq!(pane.content("tick-wait").as_deref(), Some("started"));
        assert_eq!(pane.content("ticks"), None);

        // Enabled again, the stopped run is not made again at once: the next
        // is due a minute after it began.
        block_on(launcher.set_enabled(&identity, true));
        fs::remove_file(pane.packages_dir().join("content.json")).unwrap();
        pane.clock.advance(MINUTE);
        until_seen("the next run waiting", waiting);
        assert_eq!(ticks(&launcher).last_run, Some(start() + MINUTE));

        // Reloaded while it waits: stopped, and the new code's next run is
        // due a minute after the stopped one began.
        block_on(launcher.reload(&identity));
        until(
            &launcher,
            "the run stopped by the reload",
            stopped(&format!("{title} was reloaded or updated")),
        );
        assert_eq!(
            block_on(runtime.background_running()),
            Vec::<PathBuf>::new()
        );
        assert_eq!(pane.content("ticks"), None);
        fs::remove_file(pane.packages_dir().join("content.json")).unwrap();
        pane.clock.advance(MINUTE);
        until_seen("the reloaded code's run waiting", waiting);
        assert_eq!(ticks(&launcher).last_run, Some(start() + 2 * MINUTE));

        // Turned off while it waits: stopped, and nothing more is shown.
        block_on(launcher.set_task_schedule(&command, false));
        let task = ticks(&launcher);
        assert!(!task.on && !task.running && task.last.is_none(), "{task:?}");
        until_seen("the run stopped by turning it off", || {
            block_on(runtime.background_running()).is_empty()
        });
        assert_eq!(pane.content("ticks"), None);

        // Uninstalled while it waits: stopped, and its schedule forgotten.
        block_on(launcher.set_task_schedule(&command, true));
        until(&launcher, "a run", |task| task.running);
        block_on(launcher.uninstall(&identity, SavedData::Delete));
        assert!(launcher.scheduled_tasks().is_empty());
        until_seen("the run stopped by the uninstall", || {
            block_on(runtime.background_running()).is_empty()
        });
        assert_eq!(pane.record()["tasks"], serde_json::json!({}));
    }
}

#[test]
fn a_waiting_run_holds_no_command_back_and_starts_no_other_package() {
    let pane = Pane::new();
    let (launcher, runtime) = pane.start();
    let other = Fixture {
        package: "sample-query",
        title: "Query sample",
    };
    pane.install(&launcher, &other);
    pane.install(&launcher, &RUST);
    assert_eq!(block_on(runtime.running()), Vec::<PathBuf>::new());
    choose(&launcher, "Wait 10 seconds in each run");
    let command = ticks(&launcher).command;
    block_on(launcher.set_task_schedule(&command, true));
    until_seen("the run waiting", || {
        pane.content("tick-wait").as_deref() == Some("started")
    });
    let background = block_on(runtime.background_running());
    assert_eq!(background.len(), 1, "{background:?}");
    assert!(background[0].ends_with("sample_background.wasm"));
    // Only the command opened to choose the mode runs besides the task.
    let running = block_on(runtime.running());
    assert!(
        running
            .iter()
            .all(|path| path.ends_with("sample_background.wasm")),
        "{running:?}"
    );

    // Another package's command opens while the run waits.
    let asked = Instant::now();
    launcher.back();
    launcher.back();
    block_on(launcher.set_query("echo"));
    block_on(launcher.activate_selected());
    assert!(asked.elapsed() < Duration::from_secs(5), "it waited");
    assert_eq!(launcher.view().title, "Echo: send it text from root search");
    assert!(ticks(&launcher).running);
    assert_eq!(pane.content("tick-wait").as_deref(), Some("started"));
}

#[test]
fn a_run_lost_to_a_runtime_crash_is_not_made_again_and_the_schedule_goes_on() {
    let pane = Pane::new();
    let (launcher, runtime) = pane.start();
    pane.install(&launcher, &RUST);
    choose(&launcher, "Wait 10 seconds in each run");
    let command = ticks(&launcher).command;
    block_on(launcher.set_task_schedule(&command, true));
    until_seen("the run waiting", || {
        pane.content("tick-wait").as_deref() == Some("started")
    });

    runtime.inject(pane_core::Fault::Crash);

    until(&launcher, "the lost run", |task| {
        !task.running
            && matches!(&task.last, Some(TaskOutcome::Failed(error))
                if error.starts_with("Extension runtime unavailable"))
    });
    assert_eq!(
        block_on(runtime.background_running()),
        Vec::<PathBuf>::new()
    );
    let subtitle = managed(&launcher, RUST.title);
    assert!(subtitle.starts_with("Enabled · "), "{subtitle}");
    // Not made again by itself: its next run is a minute after it began.
    choose(&launcher, "Answer normally");
    assert_eq!(ticks(&launcher).last_run, Some(start()));
    pane.clock.advance(MINUTE);
    answered(&launcher, 1);
}

#[test]
fn a_launcher_that_runs_no_background_work_explains_it() {
    let pane = Pane::new();
    let runtime = Runtime::start().unwrap();
    let launcher = Launcher::with_packages(Ok(runtime), vec![], pane.packages_dir());
    pane.install(&launcher, &RUST);
    let command = ticks(&launcher).command;

    block_on(launcher.set_task_schedule(&command, true));

    assert_eq!(
        launcher.view().status,
        Status::Error("Cannot run Ticks on a schedule: this Pane runs no background work".into())
    );
    assert!(!ticks(&launcher).on);
    assert!(!pane.packages_dir().join("schedules.json").exists());
}

impl Pane {
    fn clock_now(&self) -> SystemTime {
        use pane_core::clock::Clock;
        self.clock.now()
    }
}
