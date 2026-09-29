//! The Windows clipboard adapter against the real clipboard: it reports
//! each change the test makes, reads the markers password managers set and
//! then withholds the text, names the program owning the clipboard, puts
//! text on it, and reports nothing once its watch is dropped.
//!
//! The test replaces what is on the clipboard, and does not put it back:
//! it runs only where `PANE_TEST_REAL_CLIPBOARD=1` is set, as CI's Windows
//! runner does, never by default on a developer's computer. It uses only
//! text it puts on the clipboard itself (each starting with a prefix of its
//! own), and keeps only reports of that text, or withheld reports of this
//! process's own marked copies; anything else on the clipboard meanwhile is
//! dropped unseen. Its marked copies also say `CanIncludeInClipboardHistory`
//! 0 where the check allows, so Windows' own clipboard history (Win+V) keeps
//! them neither, and it never says a copy may be synced
//! (`CanUploadToCloudClipboard` 1). Other systems have no adapter yet
//! (their unavailability is checked in `clipboard.rs`).
#![cfg(target_os = "windows")]

use std::sync::{Mutex, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pane_core::clipboard::{
    ClipboardSystem, Content, Markers, Observation, Sink, Skip, Ticket, WindowsClipboard, accept,
    testing,
};

/// How long a change may take to be reported.
const REPORTED: Duration = Duration::from_secs(5);

const HISTORY: &str = "CanIncludeInClipboardHistory";

/// Passes on the reports of this test's own copies.
struct Ours {
    prefix: String,
    /// This test's program's file name, lowercase.
    program: String,
    reports: Mutex<mpsc::Sender<Observation>>,
}

impl Sink for Ours {
    fn reading(&self) -> Ticket {
        Ticket::default()
    }

    fn observed(&self, _: Ticket, observation: Observation) {
        let from_here = observation
            .source
            .as_deref()
            .is_some_and(|source| source.to_lowercase() == self.program);
        let ours = match &observation.content {
            Content::Text(text) => text.starts_with(&self.prefix),
            // Its text was never read: only this process's own copy can be
            // told apart, by its owner.
            Content::Withheld => from_here,
            Content::Other => false,
        };
        if ours {
            let _ = self.reports.lock().unwrap().send(observation);
        }
    }
}

#[test]
fn the_listener_reports_this_tests_changes_with_their_markers_until_dropped() {
    if std::env::var("PANE_TEST_REAL_CLIPBOARD").as_deref() != Ok("1") {
        eprintln!("skipped: set PANE_TEST_REAL_CLIPBOARD=1 to let it replace the clipboard");
        return;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let prefix = format!("pane-clipboard-test-{}-{nanos}-", std::process::id());
    let program = std::env::current_exe().unwrap();
    let program = program
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_lowercase();
    let (sender, reports) = mpsc::channel::<Observation>();
    let watch = WindowsClipboard
        .watch(std::sync::Arc::new(Ours {
            prefix: prefix.clone(),
            program: program.clone(),
            reports: Mutex::new(sender),
        }))
        .expect("Windows can watch the clipboard");
    // The next report matching `wanted`; a change can be reported more
    // than once, so an earlier one reported again is skipped.
    let next = |what: &str, wanted: &dyn Fn(&Observation) -> bool| loop {
        let report = reports
            .recv_timeout(REPORTED)
            .unwrap_or_else(|_| panic!("{what} is reported"));
        if wanted(&report) {
            return report;
        }
    };
    let text =
        |text: String| move |report: &Observation| report.content == Content::Text(text.clone());

    // Plain text, owned by a window of this test's process.
    let plain_text = format!("{prefix}plain");
    let owner = testing::set_text(&plain_text, &[]).unwrap();
    let plain = next("plain text", &text(plain_text.clone()));
    drop(owner);
    assert_eq!(accept(&plain, &[]), Ok(plain_text.as_str()));
    assert_eq!(plain.markers, Markers::default());
    assert_eq!(
        plain.source.map(|source| source.to_lowercase()),
        Some(program)
    );

    // Each marker a password manager sets is read, and withholds the text.
    let hidden = (HISTORY, 0);
    let marked: [(&[(&str, u32)], Markers); 4] = [
        (
            &[("ExcludeClipboardContentFromMonitorProcessing", 0)],
            Markers {
                exclude_from_monitoring: true,
                ..Markers::default()
            },
        ),
        (
            &[("Clipboard Viewer Ignore", 0), hidden],
            Markers {
                exclude_from_monitoring: true,
                include_in_history: Some(false),
                ..Markers::default()
            },
        ),
        (
            &[hidden],
            Markers {
                include_in_history: Some(false),
                ..Markers::default()
            },
        ),
        (
            &[("CanUploadToCloudClipboard", 0), hidden],
            Markers {
                include_in_history: Some(false),
                upload_to_cloud: Some(false),
                ..Markers::default()
            },
        ),
    ];
    for (formats, markers) in marked {
        let _owner = testing::set_text(&format!("{prefix}secret"), formats).unwrap();
        let secret = next(&format!("{formats:?}"), &|report| {
            report.content == Content::Withheld && report.markers == markers
        });
        assert!(!secret.markers.allow(), "{formats:?}");
        assert_eq!(accept(&secret, &[]), Err(Skip::Marked));
    }
    // Saying it may be kept in clipboard history (which does not sync it)
    // keeps it.
    let allowed_text = format!("{prefix}allowed");
    let _owner = testing::set_text(&allowed_text, &[(HISTORY, 1)]).unwrap();
    let allowed = next("allowed text", &text(allowed_text));
    assert_eq!(allowed.markers.include_in_history, Some(true));
    assert_eq!(allowed.markers.upload_to_cloud, None);

    // Writing is a change too.
    let written = format!("{prefix}written ✓");
    WindowsClipboard.write_text(&written).unwrap();
    next("written text", &text(written));

    // Once the watch is dropped, nothing more is reported.
    drop(watch);
    while reports.try_recv().is_ok() {}
    let _owner = testing::set_text(&format!("{prefix}after"), &[hidden]).unwrap();
    // The sink went with the watch, so the channel is closed and empty.
    assert!(matches!(
        reports.recv_timeout(Duration::from_secs(1)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    ));
}
