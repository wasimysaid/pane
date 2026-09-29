//! Pane's service sample: the smallest runnable example of a continuing
//! service. Heartbeat sets `"service": true` in `pane.json`; once the user
//! starts it in Manage extensions, Pane keeps it running in the background
//! while its package's code runs: it beats every second, counting each beat
//! in the package's content and showing "Beat N" as its status in Manage
//! extensions. Its one task is that timer, awaited in a loop, so it holds
//! nothing back while it waits.
//!
//! Lifetime: Pane starts it again whenever its package's code starts again
//! (Pane starts, the package is enabled, reloaded, updated or retried) and
//! stops it where it waits when the package is disabled, reloaded, updated,
//! paused or uninstalled, or the user stops it, so no beat is counted after
//! that. Opened from root search, the command shows the count and chooses
//! how the service behaves at its next beat, to show what Pane does with
//! each way a service ends:
//!
//! - "Keep beating": it runs on.
//! - "Stop with an error": it answers an error, an ordinary outcome shown
//!   as its failure; Pane starts it again a minute later.
//! - "Crash on purpose": it traps, a crash of the package, which Pane starts
//!   again a minute later; crashing again and again pauses the package, as
//!   any crash does.
//! - "Finish": it answers how many beats it counted, and Pane does not start
//!   it again until its package's code starts again.
#![no_std]

use pane_guest::alloc::{format, string::String, vec, vec::Vec};
use pane_guest::service::set_status;
use pane_guest::{
    CustomView, FieldValue, FormError, Guest, Item, NoCustomView, View, content, settings,
};

/// The content key holding how many beats the service counted.
const BEATS: &str = "beats";
/// The settings key holding how the service behaves at its next beat.
const MODE: &str = "beat-mode";
/// How long the service waits between beats, in nanoseconds.
const BEAT: u64 = 1_000_000_000;

struct Heartbeat;
pane_guest::export!(Heartbeat);
pane_guest::service::export!(Heartbeat);

/// The beats the service counted.
fn beats() -> Result<u64, String> {
    match content::get(BEATS)? {
        Some(count) => count
            .parse()
            .map_err(|_| "the count is not a number".into()),
        None => Ok(0),
    }
}

impl Guest for Heartbeat {
    type CustomView = NoCustomView;

    async fn get_view() -> Result<View, String> {
        let item = |id: &str, title: String, subtitle: &str| Item {
            id: id.into(),
            title,
            subtitle: Some(subtitle.into()),
            form: None,
            platforms: None,
            custom_view: None,
        };
        let mode = settings::get(MODE)?.unwrap_or_else(|| "beat".into());
        let chosen = |id: &str| if mode == id { " (chosen)" } else { "" };
        Ok(View {
            title: "Heartbeat: a continuing service".into(),
            items: vec![
                item(
                    "count",
                    format!("Beat {} times", beats()?),
                    "Start it in Manage extensions to beat every second",
                ),
                item(
                    "beat",
                    format!("Keep beating{}", chosen("beat")),
                    "It runs on, counting one beat a second",
                ),
                item(
                    "fail",
                    format!("Stop with an error{}", chosen("fail")),
                    "It fails at its next beat; Pane starts it again a minute later",
                ),
                item(
                    "crash",
                    format!("Crash on purpose{}", chosen("crash")),
                    "It crashes at its next beat; crashing again and again pauses it",
                ),
                item(
                    "finish",
                    format!("Finish{}", chosen("finish")),
                    "It finishes at its next beat, until its package starts again",
                ),
            ],
        })
    }

    async fn run_action(item_id: String) -> Result<String, String> {
        match item_id.as_str() {
            "count" => Ok(format!("Beat {} times", beats()?)),
            "beat" | "fail" | "crash" | "finish" => {
                settings::set(MODE, &item_id)?;
                Ok(match item_id.as_str() {
                    "beat" => "Heartbeat keeps beating".into(),
                    "fail" => "Heartbeat stops with an error at its next beat".into(),
                    "crash" => "Heartbeat crashes at its next beat".into(),
                    _ => "Heartbeat finishes at its next beat".into(),
                })
            }
            other => Err(format!("unknown item: {other}")),
        }
    }

    async fn submit_form(_item_id: String, _values: Vec<FieldValue>) -> Result<String, FormError> {
        Err(FormError {
            field: None,
            message: "Heartbeat has no forms".into(),
        })
    }

    async fn open_view(_item_id: String) -> Result<CustomView, String> {
        Err("Heartbeat has no custom views".into())
    }
}

impl pane_guest::service::Guest for Heartbeat {
    /// The service: a beat a second, as the chosen mode says, until Pane
    /// stops it.
    async fn run_service(command: String) -> Result<String, String> {
        if command != "heartbeat" {
            return Err(format!("unknown command: {command}"));
        }
        loop {
            match settings::get(MODE)?.as_deref() {
                Some("fail") => {
                    return Err(
                        "Heartbeat stops with an error, to show how a failed service looks".into(),
                    );
                }
                Some("crash") => panic!("Heartbeat crashes on purpose"),
                Some("finish") => return Ok(format!("Finished after {} beats", beats()?)),
                _ => {}
            }
            let count = beats()? + 1;
            content::set(BEATS, &format!("{count}"))?;
            set_status(&format!("Beat {count}"))?;
            // The service suspends here; if Pane stops it meanwhile, nothing
            // after this line runs, and no beat is counted.
            wasip3::clocks::monotonic_clock::wait_for(BEAT).await;
        }
    }
}
