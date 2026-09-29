//! Continuing services (#48) through the launcher's public interface: the
//! user starts a command's service in Manage extensions, and Pane keeps it
//! running in the background, shows its status, starts it again with its
//! package's code and after a failure, and stops it with its package's
//! code or the user's choice. Restarts after a failure are timed on a clock
//! the tests move by hand ([`ManualClock`]), so no test waits for one. The
//! service is a real guest: Heartbeat, the service sample from
//! `cargo xtask guests`, in Rust, JavaScript and TypeScript alike, which
//! beats every second.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use futures::executor::block_on;
use pane_core::clock::ManualClock;
use pane_core::{
    Launcher, PackageIdentity, RESTART_DELAY, Runtime, SavedData, Screen, ServiceOutcome,
    ServiceState, Status,
};
use serde_json::Value;
use tempfile::TempDir;

struct Fixture {
    /// The assembled package under `target/guests/packages`.
    package: &'static str,
    title: &'static str,
    /// Its component's file name.
    component: &'static str,
}

const RUST: Fixture = Fixture {
    package: "sample-service",
    title: "Service sample",
    component: "sample_service.wasm",
};
const JAVASCRIPT: Fixture = Fixture {
    package: "sample-service-js",
    title: "JavaScript service sample",
    component: "sample_service_js.wasm",
};
const TYPESCRIPT: Fixture = Fixture {
    package: "sample-service-ts",
    title: "TypeScript service sample",
    component: "sample_service_ts.wasm",
};
const ALL: [Fixture; 3] = [RUST, JAVASCRIPT, TYPESCRIPT];

/// When the tests' clock starts.
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

    /// How many beats the service counted, as its package keeps them.
    fn beats(&self) -> u64 {
        let Ok(text) = fs::read_to_string(self.packages_dir().join("content.json")) else {
            return 0;
        };
        let content: Value = serde_json::from_str(&text).unwrap();
        content["packages"]
            .as_object()
            .and_then(|packages| {
                packages
                    .values()
                    .find_map(|values| values.get("beats")?.as_str()?.parse().ok())
            })
            .unwrap_or(0)
    }

    /// The record of started services, as written.
    fn record(&self) -> Value {
        let text = fs::read_to_string(self.packages_dir().join("services.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }
}

/// The one service, Heartbeat.
fn heartbeat(launcher: &Launcher) -> ServiceState {
    let services = launcher.services();
    assert_eq!(services.len(), 1, "{services:?}");
    services.into_iter().next().unwrap()
}

/// Waits, on the launcher's background changes, until `done` holds.
fn until(launcher: &Launcher, what: &str, done: impl Fn(&ServiceState) -> bool) {
    let mut changes = launcher.background_changes();
    let waiting = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done(&heartbeat(launcher)) {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "{what} did not happen: {:?}",
            heartbeat(launcher)
        );
        let _ = waiting.block_on(async { tokio::time::timeout(left, changes.next()).await });
    }
}

/// Waits until the service runs and shows a beat.
fn beating(launcher: &Launcher) {
    until(launcher, "a beat", |service| {
        service.running
            && service
                .status
                .as_deref()
                .is_some_and(|status| status.starts_with("Beat "))
    });
}

/// Opens Heartbeat and runs its item titled `title` (how the service
/// behaves at its next beat).
fn choose(launcher: &Launcher, title: &str) {
    launcher.back();
    launcher.back();
    block_on(launcher.set_query("heartbeat"));
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

/// The components whose background work runs.
fn background(runtime: &Runtime) -> Vec<PathBuf> {
    block_on(runtime.background_running())
}

/// Waits until no background work runs.
fn released(runtime: &Runtime) {
    let started = Instant::now();
    while !background(runtime).is_empty() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the service's instance was not released"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn installing_starts_nothing_and_a_started_service_shows_its_status_until_stopped() {
    for fixture in ALL {
        let pane = Pane::new();
        let (launcher, runtime) = pane.start();
        pane.install(&launcher, &fixture);

        let service = heartbeat(&launcher);
        assert_eq!(service.title, "Heartbeat");
        assert!(!service.on && !service.running, "{service:?}");
        let subtitle = managed(&launcher, "Service: Heartbeat");
        assert!(subtitle.starts_with("Stopped · Start it"), "{subtitle}");
        pane.clock.advance(10 * RESTART_DELAY);
        assert_eq!(background(&runtime), Vec::<PathBuf>::new());
        assert_eq!(pane.beats(), 0, "installing started it");

        // Started in Manage extensions (Enter on its row): it runs at once,
        // and its status is shown there.
        let row = launcher
            .view()
            .rows
            .iter()
            .position(|row| row.title == "Service: Heartbeat")
            .unwrap();
        launcher.select(row);
        block_on(launcher.activate_selected());
        assert_eq!(
            launcher.view().status,
            Status::Result("Heartbeat runs in the background from now on".into())
        );
        beating(&launcher);
        let subtitle = managed(&launcher, "Service: Heartbeat");
        assert!(subtitle.starts_with("Running · Beat "), "{subtitle}");
        let running = background(&runtime);
        assert_eq!(running.len(), 1, "{running:?}");
        assert!(running[0].ends_with(fixture.component));
        let record = pane.record();
        assert_eq!(
            record["services"],
            serde_json::json!([heartbeat(&launcher).command]),
            "{record}"
        );
        // Its status follows its beats.
        let first = pane.beats();
        until(&launcher, "a later beat", |service| {
            service.status.as_deref() == Some(&format!("Beat {}", first + 1))
        });

        // Stopped: its instance goes, and its status with it.
        block_on(launcher.set_service(&heartbeat(&launcher).command, false));
        let service = heartbeat(&launcher);
        assert!(!service.on && !service.running && service.status.is_none());
        released(&runtime);
        let subtitle = managed(&launcher, "Service: Heartbeat");
        assert!(subtitle.starts_with("Stopped · "), "{subtitle}");
        assert_eq!(pane.record()["services"], serde_json::json!([]));
        pane.clock.advance(10 * RESTART_DELAY);
        assert!(!heartbeat(&launcher).running);
    }
}

#[test]
fn a_started_service_starts_again_with_pane_and_with_its_packages_code() {
    for fixture in ALL {
        let pane = Pane::new();
        let identity = {
            let (launcher, _runtime) = pane.start();
            let identity = pane.install(&launcher, &fixture);
            block_on(launcher.set_service(&heartbeat(&launcher).command, true));
            beating(&launcher);
            identity
        };

        // Pane started again: so is the service.
        let (launcher, runtime) = pane.start();
        beating(&launcher);
        assert_eq!(background(&runtime).len(), 1);

        // Disabled: stopped where it waits, its instance released.
        block_on(launcher.set_enabled(&identity, false));
        until(&launcher, "the stop", |service| !service.running);
        assert_eq!(heartbeat(&launcher).status, None);
        released(&runtime);
        let title = fixture.title;
        assert_eq!(
            heartbeat(&launcher).last,
            Some(ServiceOutcome::Stopped(format!("{title} was disabled")))
        );
        let beats = pane.beats();
        // No restart is due while it is disabled, however long.
        pane.clock.advance(10 * RESTART_DELAY);
        assert!(!heartbeat(&launcher).running);

        // Enabled: started again at once, as one run.
        block_on(launcher.set_enabled(&identity, true));
        until(&launcher, "a beat after enabling", |service| {
            service.status.as_deref() == Some(&format!("Beat {}", beats + 1))
        });
        assert_eq!(background(&runtime).len(), 1);

        // Reloaded: the old code's run stops, the new code's starts at once.
        let beats = pane.beats();
        block_on(launcher.reload(&identity));
        until(&launcher, "a beat of the reloaded code", |service| {
            service.running
                && service.status.as_deref().is_some_and(|status| {
                    status
                        .strip_prefix("Beat ")
                        .and_then(|count| count.parse::<u64>().ok())
                        .is_some_and(|count| count > beats)
                })
        });
        assert_eq!(background(&runtime).len(), 1, "the old run went on");

        // Uninstalled: stopped, released and forgotten.
        block_on(launcher.uninstall(&identity, SavedData::Delete));
        assert!(launcher.services().is_empty());
        released(&runtime);
        assert_eq!(pane.record()["services"], serde_json::json!([]));
    }
}

#[test]
fn an_error_restarts_it_a_minute_later_and_repeated_crashes_pause_the_package() {
    for fixture in ALL {
        let pane = Pane::new();
        let (launcher, runtime) = pane.start();
        let identity = pane.install(&launcher, &fixture);
        choose(&launcher, "Stop with an error");
        block_on(launcher.set_service(&heartbeat(&launcher).command, true));
        let failed = |service: &ServiceState| {
            !service.running
                && matches!(&service.last, Some(ServiceOutcome::Failed(error))
                if error.contains("Heartbeat stops with an error, to show how a failed service looks"))
        };
        until(&launcher, "a failure", failed);
        let ended = heartbeat(&launcher).restart_at.expect("a restart");
        let subtitle = managed(&launcher, "Service: Heartbeat");
        assert!(
            subtitle.starts_with("Failed: ")
                && subtitle.contains("It starts again a minute after it ended"),
            "{subtitle}"
        );
        assert_eq!(background(&runtime), Vec::<PathBuf>::new());

        // Not before the minute is up; then again, and again: an error never
        // pauses it.
        pane.clock.advance(RESTART_DELAY - Duration::from_secs(1));
        assert_eq!(heartbeat(&launcher).restart_at, Some(ended));
        for _ in 0..3 {
            let before = heartbeat(&launcher).restart_at;
            pane.clock.advance(RESTART_DELAY);
            until(&launcher, "another failure", |service| {
                failed(service) && service.restart_at != before
            });
        }
        let subtitle = managed(&launcher, fixture.title);
        assert!(subtitle.starts_with("Enabled · "), "{subtitle}");

        // Crashes count as any crash does: the third within five minutes
        // pauses the package, and the service waits for Retry.
        choose(&launcher, "Crash on purpose");
        for crash in 1..=3 {
            let before = heartbeat(&launcher).restart_at;
            pane.clock.advance(RESTART_DELAY);
            until(&launcher, &format!("crash {crash}"), |service| {
                !service.running
                    && service.restart_at != before
                    && matches!(service.last, Some(ServiceOutcome::Crashed(_)))
            });
        }
        let subtitle = managed(&launcher, fixture.title);
        assert!(
            subtitle.starts_with("Enabled · Paused after crashing"),
            "{subtitle}"
        );
        let subtitle = managed(&launcher, "Service: Heartbeat");
        assert!(subtitle.starts_with("Not running: "), "{subtitle}");
        pane.clock.advance(10 * RESTART_DELAY);
        assert!(!heartbeat(&launcher).running, "a paused service started");
        assert_eq!(background(&runtime), Vec::<PathBuf>::new());

        // Retry starts it at once (and it crashes once more, as chosen).
        let before = heartbeat(&launcher).restart_at;
        block_on(launcher.retry_start(&identity));
        until(&launcher, "a start after Retry", |service| {
            service.restart_at != before
        });
    }
}

#[test]
fn a_finished_service_waits_for_its_packages_code_to_start_again() {
    let pane = Pane::new();
    let (launcher, _runtime) = pane.start();
    let identity = pane.install(&launcher, &RUST);
    choose(&launcher, "Finish");
    block_on(launcher.set_service(&heartbeat(&launcher).command, true));
    until(&launcher, "the finish", |service| {
        !service.running && matches!(service.last, Some(ServiceOutcome::Finished(_)))
    });
    let service = heartbeat(&launcher);
    assert_eq!(
        service.last,
        Some(ServiceOutcome::Finished("Finished after 0 beats".into()))
    );
    assert_eq!(service.restart_at, None);
    let subtitle = managed(&launcher, "Service: Heartbeat");
    assert!(
        subtitle.starts_with("Finished: Finished after 0 beats"),
        "{subtitle}"
    );
    pane.clock.advance(10 * RESTART_DELAY);
    assert!(!heartbeat(&launcher).running);

    // Its package's code starting again starts it again.
    choose(&launcher, "Keep beating");
    block_on(launcher.set_enabled(&identity, false));
    block_on(launcher.set_enabled(&identity, true));
    beating(&launcher);
}

#[test]
fn a_running_service_holds_no_command_back_and_starts_no_other_package() {
    let pane = Pane::new();
    let (launcher, runtime) = pane.start();
    let other = Fixture {
        package: "sample-query",
        title: "Query sample",
        component: "sample_query.wasm",
    };
    pane.install(&launcher, &other);
    pane.install(&launcher, &RUST);
    block_on(launcher.set_service(&heartbeat(&launcher).command, true));
    beating(&launcher);
    assert_eq!(block_on(runtime.running()), Vec::<PathBuf>::new());

    let asked = Instant::now();
    launcher.back();
    launcher.back();
    block_on(launcher.set_query("echo"));
    block_on(launcher.activate_selected());
    assert!(asked.elapsed() < Duration::from_secs(5), "it waited");
    assert_eq!(launcher.view().title, "Echo: send it text from root search");
    let running = block_on(runtime.running());
    assert!(
        running.iter().all(|path| path.ends_with(other.component)),
        "{running:?}"
    );
    assert!(heartbeat(&launcher).running);
}

#[test]
fn a_service_lost_to_a_runtime_crash_starts_again_a_minute_later() {
    let pane = Pane::new();
    let (launcher, runtime) = pane.start();
    pane.install(&launcher, &RUST);
    block_on(launcher.set_service(&heartbeat(&launcher).command, true));
    beating(&launcher);

    runtime.inject(pane_core::Fault::Crash);

    until(&launcher, "the lost service", |service| {
        !service.running
            && matches!(&service.last, Some(ServiceOutcome::Failed(error))
                if error.starts_with("Extension runtime unavailable"))
    });
    let subtitle = managed(&launcher, RUST.title);
    assert!(subtitle.starts_with("Enabled · "), "{subtitle}");
    pane.clock.advance(RESTART_DELAY);
    beating(&launcher);
}

#[test]
fn a_launcher_that_runs_no_background_work_explains_it() {
    let pane = Pane::new();
    let runtime = Runtime::start().unwrap();
    let launcher = Launcher::with_packages(Ok(runtime), vec![], pane.packages_dir());
    pane.install(&launcher, &RUST);

    block_on(launcher.set_service(&heartbeat(&launcher).command, true));

    assert_eq!(
        launcher.view().status,
        Status::Error("Cannot start Heartbeat: this Pane runs no background work".into())
    );
    assert!(!heartbeat(&launcher).on);
    assert!(!pane.packages_dir().join("services.json").exists());
}
