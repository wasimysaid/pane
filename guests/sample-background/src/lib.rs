//! Pane's background sample: the smallest runnable example of a scheduled
//! task. Ticks declares a schedule in `pane.json` (every minute); once the
//! user turns it on in Manage extensions, Pane runs its task in the
//! background: each run adds one to a count kept in the package's content
//! and answers "Ticked N times", which Manage extensions shows as the
//! task's latest result. Opened from root search, the command shows the
//! count and chooses how the next runs behave, to show what Pane does with
//! each outcome:
//!
//! - "Answer normally": counts.
//! - "Answer with an error": the run answers an error, an ordinary outcome
//!   shown as the latest failure; the schedule goes on.
//! - "Crash on purpose": the run traps, a crash of the package; runs that
//!   crash again and again pause it, as any crash does.
//! - "Wait 10 seconds in each run": the run notes "started", waits, then
//!   notes "finished" and counts; disabling or reloading the package, or
//!   turning the schedule off, meanwhile stops it where it waits, so
//!   "finished" is never noted.
//!
//! Each run starts in an instance of its own: what the next run needs is
//! kept in extension data, not in memory.
#![no_std]

use pane_guest::alloc::{format, string::String, vec, vec::Vec};
use pane_guest::{
    CustomView, FieldValue, FormError, Guest, Item, NoCustomView, View, content, settings,
};

/// The content key holding how many runs counted.
const TICKS: &str = "ticks";
/// The settings key holding how the next runs behave.
const MODE: &str = "tick-mode";
/// The content key where a waiting run notes how far it got.
const WAITED: &str = "tick-wait";
/// How long a waiting run waits, in nanoseconds.
const WAIT: u64 = 10_000_000_000;

struct Ticks;
pane_guest::export!(Ticks);
pane_guest::scheduled::export!(Ticks);

/// The count the runs kept.
fn ticks() -> Result<u64, String> {
    match content::get(TICKS)? {
        Some(count) => count
            .parse()
            .map_err(|_| "the count is not a number".into()),
        None => Ok(0),
    }
}

/// Adds one to the count, and answers the new count.
fn tick() -> Result<String, String> {
    let count = ticks()? + 1;
    content::set(TICKS, &format!("{count}"))?;
    Ok(format!("Ticked {count} times"))
}

impl Guest for Ticks {
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
        let mode = settings::get(MODE)?.unwrap_or_else(|| "answer".into());
        let chosen = |id: &str| if mode == id { " (chosen)" } else { "" };
        Ok(View {
            title: "Ticks: a scheduled task".into(),
            items: vec![
                item(
                    "count",
                    format!("Ticked {} times", ticks()?),
                    "Turn its schedule on in Manage extensions to count every minute",
                ),
                item(
                    "answer",
                    format!("Answer normally{}", chosen("answer")),
                    "Each run counts one more",
                ),
                item(
                    "fail",
                    format!("Answer with an error{}", chosen("fail")),
                    "Each run answers an error; the schedule goes on",
                ),
                item(
                    "crash",
                    format!("Crash on purpose{}", chosen("crash")),
                    "Each run crashes; crashing again and again pauses it",
                ),
                item(
                    "wait",
                    format!("Wait 10 seconds in each run{}", chosen("wait")),
                    "Disabling or reloading it meanwhile stops the run",
                ),
            ],
        })
    }

    async fn run_action(item_id: String) -> Result<String, String> {
        match item_id.as_str() {
            "count" => Ok(format!("Ticked {} times", ticks()?)),
            "answer" | "fail" | "crash" | "wait" => {
                settings::set(MODE, &item_id)?;
                Ok(match item_id.as_str() {
                    "answer" => "The next runs count".into(),
                    "fail" => "The next runs answer an error".into(),
                    "crash" => "The next runs crash".into(),
                    _ => "The next runs wait 10 seconds, then count".into(),
                })
            }
            other => Err(format!("unknown item: {other}")),
        }
    }

    async fn submit_form(_item_id: String, _values: Vec<FieldValue>) -> Result<String, FormError> {
        Err(FormError {
            field: None,
            message: "Ticks has no forms".into(),
        })
    }

    async fn open_view(_item_id: String) -> Result<CustomView, String> {
        Err("Ticks has no custom views".into())
    }
}

impl pane_guest::scheduled::Guest for Ticks {
    /// One run of Ticks' task, as the chosen mode says.
    async fn run_task(command: String) -> Result<String, String> {
        if command != "ticks" {
            return Err(format!("unknown command: {command}"));
        }
        match settings::get(MODE)?.as_deref() {
            Some("fail") => Err("Ticks refuses to count, to show how a failed run looks".into()),
            Some("crash") => panic!("Ticks crashes on purpose"),
            Some("wait") => {
                content::set(WAITED, "started")?;
                // The run suspends here; if Pane stops it meanwhile, nothing
                // after this line runs.
                wasip3::clocks::monotonic_clock::wait_for(WAIT).await;
                content::set(WAITED, "finished")?;
                tick()
            }
            _ => tick(),
        }
    }
}
