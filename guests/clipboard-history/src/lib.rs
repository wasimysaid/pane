//! Pane's clipboard history, a default extension: once the user turns it
//! on in its command, Pane keeps the text they copy on this computer, and
//! the command lists it, newest first; Enter on an item copies it again.
//! It starts off, and can be paused, resumed and turned off again; disabling
//! the extension stops it too. Programs can be excluded by their file name.
//!
//! Pane's host does the watching and keeping (`pane:extension/clipboard-history`):
//! this command only shows the history and the user's controls, and nothing
//! of it runs while the clipboard changes.
#![no_std]

use pane_guest::alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use pane_guest::clipboard_history::{self as history, Capture, Entry, HistoryStatus};
use pane_guest::{
    CustomView, Field, FieldKind, FieldValue, Form, FormError, Guest, Item, NoCustomView,
    TextField, View,
};

struct ClipboardHistory;
pane_guest::export!(ClipboardHistory);

// The items that turn keeping on, pause it, resume it and turn it off. Each
// does only that, so pressing one again before the view is shown anew
// changes nothing more.
const TURN_ON: &str = "turn-on";
const PAUSE: &str = "pause";
const RESUME: &str = "resume";
const TURN_OFF: &str = "turn-off";
/// The item whose form excludes a program.
const EXCLUDE: &str = "exclude";
/// The prefix of the item that no longer excludes the program named by the
/// rest.
const INCLUDE: &str = "include:";
/// The item that deletes every kept item.
const CLEAR: &str = "clear";
/// The prefix of a kept item, followed by its id.
const ENTRY: &str = "entry:";
/// The item shown while nothing is kept.
const EMPTY: &str = "empty";

/// The longest title of a kept item, in characters.
const TITLE_CHARS: usize = 80;

fn item(id: &str, title: String, subtitle: String) -> Item {
    Item {
        id: id.into(),
        title,
        subtitle: Some(subtitle),
        form: None,
        platforms: None,
        custom_view: None,
    }
}

fn plural(count: u32, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

/// The item that changes whether Pane keeps what is copied.
fn toggle(status: &HistoryStatus) -> Item {
    let (id, title, subtitle) = match status.capture {
        Capture::Off => (
            TURN_ON,
            "Turn on clipboard history",
            "Off · Pane keeps nothing you copy until you turn it on. Once on, it keeps the text \
             you copy on this computer; nothing is sent anywhere"
                .to_string(),
        ),
        Capture::On => (
            PAUSE,
            "Pause clipboard history",
            format!(
                "On · {} kept · Text you copy is kept on this computer",
                plural(status.items, "item", "items")
            ),
        ),
        Capture::Paused => (
            RESUME,
            "Resume clipboard history",
            format!(
                "Paused · {} kept · Nothing you copy is kept until you resume",
                plural(status.items, "item", "items")
            ),
        ),
    };
    let subtitle = match &status.problem {
        Some(problem) => format!("{problem} · {subtitle}"),
        None => subtitle,
    };
    item(id, title.into(), subtitle)
}

fn exclude_form() -> Form {
    Form {
        title: "Exclude a program".into(),
        fields: vec![Field {
            id: "program".into(),
            label: "Program file name".into(),
            kind: FieldKind::Text(TextField {
                placeholder: Some("KeePass.exe".into()),
            }),
        }],
        submit_label: "Exclude".into(),
    }
}

/// The first line of `text` with content, trimmed and at most
/// [`TITLE_CHARS`] long.
fn title_of(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if line.chars().count() <= TITLE_CHARS {
        return line.into();
    }
    let mut title: String = line.chars().take(TITLE_CHARS - 1).collect();
    title.push('…');
    title
}

/// "just now", "5 min ago", "3 h ago", "2 days ago".
fn age(seconds: u64) -> String {
    match seconds {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86400 => format!("{} h ago", seconds / 3600),
        _ if seconds < 2 * 86400 => "1 day ago".into(),
        _ => format!("{} days ago", seconds / 86400),
    }
}

fn entry_item(entry: &Entry) -> Item {
    let mut about = vec![age(entry.age_seconds)];
    if let Some(source) = &entry.source {
        about.push(format!("from {source}"));
    }
    let lines = entry.text.lines().count();
    if lines > 1 {
        about.push(format!("{lines} lines"));
    }
    about.push("Enter copies it".into());
    item(
        &format!("{ENTRY}{}", entry.id),
        title_of(&entry.text),
        about.join(" · "),
    )
}

fn form_error(message: String) -> FormError {
    FormError {
        field: None,
        message,
    }
}

impl Guest for ClipboardHistory {
    type CustomView = NoCustomView;

    async fn get_view() -> Result<View, String> {
        let status = history::status()?;
        let mut items = vec![toggle(&status)];
        if status.capture != Capture::Off {
            items.push(item(
                TURN_OFF,
                "Turn off clipboard history".into(),
                "Stops keeping what you copy; the kept items stay until you clear them".into(),
            ));
        }
        let excluded = match status.excluded.len() {
            0 => "None excluded".to_string(),
            count => format!("{count} excluded"),
        };
        items.push(Item {
            form: Some(exclude_form()),
            ..item(
                EXCLUDE,
                "Exclude a program".into(),
                format!("Text copied from it is never kept · {excluded}"),
            )
        });
        items.extend(status.excluded.iter().map(|program| {
            item(
                &format!("{INCLUDE}{program}"),
                format!("Stop excluding {program}"),
                format!("Text copied from {program} is not kept"),
            )
        }));
        let entries = history::entries()?;
        if !entries.is_empty() {
            items.push(item(
                CLEAR,
                "Clear clipboard history".into(),
                format!(
                    "Deletes the {} kept; whether history is kept does not change",
                    plural(status.items, "item", "items")
                ),
            ));
        }
        items.extend(entries.iter().map(entry_item));
        if entries.is_empty() && status.capture == Capture::On {
            items.push(item(
                EMPTY,
                "Nothing kept yet".into(),
                "Text you copy from now on is listed here".into(),
            ));
        }
        Ok(View {
            title: "Clipboard History".into(),
            items,
        })
    }

    async fn run_action(item_id: String) -> Result<String, String> {
        let wanted = match item_id.as_str() {
            TURN_ON => Some((Capture::On, "Clipboard history is on")),
            PAUSE => Some((Capture::Paused, "Clipboard history is paused")),
            RESUME => Some((Capture::On, "Clipboard history is on again")),
            TURN_OFF => Some((Capture::Off, "Clipboard history is off")),
            _ => None,
        };
        if let Some((capture, done)) = wanted {
            history::set_capture(capture)?;
            return Ok(done.into());
        }
        if item_id == CLEAR {
            let cleared = history::clear()?;
            return Ok(format!(
                "Deleted {}",
                plural(cleared, "kept item", "kept items")
            ));
        }
        if item_id == EMPTY {
            return Ok("Nothing is kept yet".into());
        }
        if let Some(program) = item_id.strip_prefix(INCLUDE) {
            let mut excluded = history::status()?.excluded;
            excluded.retain(|excluded| excluded != program);
            history::set_excluded(&excluded)?;
            return Ok(format!("Text copied from {program} is kept again"));
        }
        if let Some(id) = item_id.strip_prefix(ENTRY) {
            history::copy(id)?;
            return Ok("Copied to the clipboard".into());
        }
        Err(format!("unknown item: {item_id}"))
    }

    async fn submit_form(item_id: String, values: Vec<FieldValue>) -> Result<String, FormError> {
        if item_id != EXCLUDE {
            return Err(form_error(format!("unknown form: {item_id}")));
        }
        let program = values
            .iter()
            .find(|value| value.id == "program")
            .map_or("", |value| value.value.trim());
        let mut excluded = history::status().map_err(form_error)?.excluded;
        excluded.push(program.into());
        history::set_excluded(&excluded).map_err(|message| FormError {
            field: Some("program".into()),
            message,
        })?;
        Ok(format!("Text copied from {program} is not kept"))
    }

    async fn open_view(item_id: String) -> Result<CustomView, String> {
        Err(format!("unknown view: {item_id}"))
    }
}
