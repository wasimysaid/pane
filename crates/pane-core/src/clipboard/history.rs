//! The clipboard history file, `clipboard-history.json`: every package's
//! history under its package identity's key, typed and versioned.
//!
//! ```json
//! { "version": 1, "packages": { "local:/…": {
//!     "capture": "on",
//!     "excluded": ["keepass.exe"],
//!     "items": [{ "id": 7, "text": "hello", "copiedAt": 1790000000000, "source": "notepad.exe" }],
//!     "nextId": 8 } } }
//! ```
//!
//! Changes are made in memory under the store's lock and written after it
//! is released, so a copy never waits for another's write while holding it,
//! and a write never holds up a capture. Writes are made one at a time, and
//! a write older than the last one written is skipped, so the file always
//! ends with the latest state. A change is on disk once the call that made
//! it returns; a crash before that loses it (the file is replaced
//! atomically, so it holds the state before or after, never a torn one).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use super::{CaptureState, MAX_EXCLUDED, MAX_ITEMS, ProgramName};
use crate::atomic::{Readers, write_atomically};
use crate::extension_data::Removal;

/// The history file's name, beside `installed.json`.
pub(crate) const FILE: &str = "clipboard-history.json";

/// The version of the history file's format.
const VERSION: u64 = 1;

/// One kept text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    /// Identifies it among the package's items; later items have greater
    /// ids.
    pub id: u64,
    pub text: String,
    /// When it was copied, in milliseconds since the Unix epoch.
    pub copied_at: u64,
    /// The program it was copied from, if the system said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// One package's clipboard history.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackageHistory {
    #[serde(default, skip_serializing_if = "CaptureState::is_off")]
    pub capture: CaptureState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded: Vec<ProgramName>,
    /// Newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Item>,
    /// The id the next item gets.
    #[serde(default, skip_serializing_if = "is_zero")]
    next_id: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

impl PackageHistory {
    /// Whether nothing is kept: no items, and every choice as it starts.
    pub fn is_empty(&self) -> bool {
        self.capture.is_off() && self.excluded.is_empty() && self.items.is_empty()
    }

    /// Keeps `text`, copied from `source` at `now`, as the newest item: an
    /// item with the same text moves to the front instead of being kept
    /// twice, and beyond [`MAX_ITEMS`] the oldest go.
    pub fn add(&mut self, text: &str, source: Option<&str>, now: u64) {
        self.items.retain(|item| item.text != text);
        let id = self
            .next_id
            .max(self.items.iter().map(|item| item.id + 1).max().unwrap_or(0));
        self.next_id = id + 1;
        self.items.insert(
            0,
            Item {
                id,
                text: text.to_owned(),
                copied_at: now,
                source: source.map(str::to_owned),
            },
        );
        self.items.truncate(MAX_ITEMS);
    }

    /// Removes every item, keeping the capture state and the excluded
    /// programs; returns how many there were.
    pub fn clear(&mut self) -> usize {
        std::mem::take(&mut self.items).len()
    }

    /// Replaces the excluded programs with `programs`, each once, in the
    /// order given; or why one is not a program's file name.
    pub fn set_excluded(&mut self, programs: &[String]) -> Result<(), String> {
        let mut kept: Vec<ProgramName> = Vec::new();
        for program in programs {
            let program = ProgramName::parse(program)?;
            if !kept.contains(&program) {
                kept.push(program);
            }
        }
        if kept.len() > MAX_EXCLUDED {
            return Err(format!("At most {MAX_EXCLUDED} programs can be excluded"));
        }
        self.excluded = kept;
        Ok(())
    }
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
struct HistoryJson {
    version: u64,
    packages: BTreeMap<String, PackageHistory>,
}

/// The file as Pane last read or changed it.
struct State {
    file: Result<HistoryJson, String>,
    /// Counts changes, so that an older write is never made after a newer.
    changes: u64,
    /// Counts the times items were deleted (Clear, uninstall): a capture
    /// that began reading before one keeps nothing (see [`Ticket`]).
    ///
    /// [`Ticket`]: super::Ticket
    deletions: u64,
}

/// Every package's clipboard history, kept in one file.
pub(crate) struct HistoryStore {
    path: PathBuf,
    state: Mutex<State>,
    /// The number of the last change written, held while writing.
    written: Mutex<u64>,
}

/// A change made in memory, to be written once the store is unlocked.
pub(crate) struct Pending {
    change: u64,
    file: HistoryJson,
}

impl HistoryStore {
    /// Opens the history kept in `dir`. Nothing is written until something
    /// changes.
    pub fn open(dir: &Path) -> HistoryStore {
        let path = dir.join(FILE);
        let file = read(&path);
        HistoryStore {
            path,
            state: Mutex::new(State {
                file,
                changes: 0,
                deletions: 0,
            }),
            written: Mutex::new(0),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The history of `owner`, or why it cannot be read.
    pub fn get(&self, owner: &str) -> Result<PackageHistory, String> {
        let state = self.lock();
        let file = state.file.as_ref().map_err(Clone::clone)?;
        Ok(file.packages.get(owner).cloned().unwrap_or_default())
    }

    /// The owners whose capture is on.
    pub fn capturing_owners(&self) -> Vec<String> {
        let state = self.lock();
        let Ok(file) = &state.file else {
            return Vec::new();
        };
        file.packages
            .iter()
            .filter(|(_, history)| history.capture == CaptureState::On)
            .map(|(owner, _)| owner.clone())
            .collect()
    }

    /// How many times items were deleted.
    pub fn deletions(&self) -> u64 {
        self.lock().deletions
    }

    /// Hands `change` the history of `owner` and keeps what it changed,
    /// written before this returns. Returns its answer and whether the
    /// capture state changed.
    pub fn update<R>(
        &self,
        owner: &str,
        change: impl FnOnce(&mut PackageHistory) -> Result<R, String>,
    ) -> Result<(R, bool), String> {
        let (answer, capture_changed, pending) = {
            let mut state = self.lock();
            let file = state.file.as_ref().map_err(Clone::clone)?;
            let before = file.packages.get(owner).cloned().unwrap_or_default();
            let mut history = before.clone();
            let answer = change(&mut history)?;
            if history == before {
                return Ok((answer, false));
            }
            if history.items.len() < before.items.len() {
                state.deletions += 1;
            }
            let capture_changed = history.capture != before.capture;
            let pending = state.change(|file| {
                if history.is_empty() {
                    file.packages.remove(owner);
                } else {
                    file.packages.insert(owner.to_owned(), history);
                }
            });
            (answer, capture_changed, pending)
        };
        self.write(pending)
            .map_err(|error| format!("Could not save the clipboard history: {error}"))?;
        Ok((answer, capture_changed))
    }

    /// Changes every history `change` asks to, if items were not deleted
    /// since `deletions`, then writes them; a failed write is reported on
    /// standard error, never with what was copied.
    pub fn capture(
        &self,
        deletions: u64,
        change: impl FnOnce(&mut BTreeMap<String, PackageHistory>) -> bool,
    ) {
        let pending = {
            let mut state = self.lock();
            if state.deletions != deletions {
                return;
            }
            let Ok(file) = &state.file else {
                return;
            };
            let mut packages = file.packages.clone();
            if !change(&mut packages) {
                return;
            }
            state.change(|file| file.packages = packages)
        };
        if let Err(error) = self.write(pending) {
            eprintln!(
                "Pane could not keep a copied text in {}: {error}",
                self.path.display()
            );
        }
    }

    /// Removes the history of `owner`, and nothing else; the file is read
    /// again first if it could not be read before, so one the user repaired
    /// is used without restarting Pane. On failure nothing is removed.
    pub fn remove(&self, owner: &str) -> Result<(), Removal> {
        let pending = {
            let mut state = self.lock();
            if state.file.is_err() {
                state.file = read(&self.path);
            }
            let file = state
                .file
                .as_ref()
                .map_err(|reason| Removal::Unreadable(reason.clone()))?;
            if !file.packages.contains_key(owner) {
                return Ok(());
            }
            state.deletions += 1;
            state.change(|file| {
                file.packages.remove(owner);
            })
        };
        self.write(pending)
            .map_err(|error| Removal::Unwritable(self.path.clone(), error))
    }

    /// How many items each owner keeps, read from the file now (an owner
    /// that keeps only its choices counts 0), or why it cannot be read.
    pub fn counts_now(&self) -> Result<BTreeMap<String, usize>, String> {
        Ok(read(&self.path)?
            .packages
            .into_iter()
            .map(|(owner, history)| (owner, history.items.len()))
            .collect())
    }

    /// Writes `pending`, unless a newer change was written meanwhile.
    fn write(&self, pending: Pending) -> io::Result<()> {
        let mut written = self
            .written
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if pending.change <= *written {
            return Ok(());
        }
        let text = serde_json::to_string_pretty(&pending.file).map_err(io::Error::other)?;
        write_atomically(&self.path, text.as_bytes(), Readers::OwnerOnly)?;
        *written = pending.change;
        Ok(())
    }
}

impl State {
    /// Applies `change` to the file in memory, returning it to be written.
    fn change(&mut self, change: impl FnOnce(&mut HistoryJson)) -> Pending {
        let file = self
            .file
            .as_mut()
            .expect("only a file that could be read is changed");
        change(file);
        self.changes += 1;
        Pending {
            change: self.changes,
            file: file.clone(),
        }
    }
}

/// Reads the history file; a missing file holds none.
fn read(path: &Path) -> Result<HistoryJson, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<HistoryJson>(&text)
            .map_err(|error| error.to_string())
            .and_then(|file| {
                if file.version == VERSION {
                    Ok(file)
                } else {
                    Err(format!(
                        "it has version {}, this Pane reads {VERSION}",
                        file.version
                    ))
                }
            }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(HistoryJson {
            version: VERSION,
            packages: BTreeMap::new(),
        }),
        Err(error) => Err(error.to_string()),
    }
    .map_err(|reason| format!("Cannot read {}: {reason}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(history: &PackageHistory) -> Vec<&str> {
        history
            .items
            .iter()
            .map(|item| item.text.as_str())
            .collect()
    }

    #[test]
    fn items_are_newest_first_once_per_text_and_bounded() {
        let mut history = PackageHistory::default();
        history.add("one", Some("notepad.exe"), 10);
        history.add("two", None, 20);
        assert_eq!(texts(&history), ["two", "one"]);
        assert_eq!(history.items[1].source.as_deref(), Some("notepad.exe"));
        assert_eq!(history.items[1].copied_at, 10);
        // Copying "one" again moves it to the front, with its new time.
        history.add("one", None, 30);
        assert_eq!(texts(&history), ["one", "two"]);
        assert_eq!(history.items[0].copied_at, 30);
        assert!(history.items[0].id > history.items[1].id);
        for number in 0..MAX_ITEMS {
            history.add(&format!("item {number}"), None, 40);
        }
        assert_eq!(history.items.len(), MAX_ITEMS);
        assert_eq!(history.items[0].text, format!("item {}", MAX_ITEMS - 1));
        assert!(!texts(&history).contains(&"two") && !texts(&history).contains(&"one"));
        // Ids are never reused, even after clearing.
        let last = history.items[0].id;
        history.clear();
        history.add("again", None, 50);
        assert!(history.items[0].id > last);
    }

    #[test]
    fn capture_state_and_exclusions_are_kept_apart_from_items() {
        let mut history = PackageHistory {
            capture: CaptureState::Paused,
            ..PackageHistory::default()
        };
        let programs = [
            "KeePass.exe".to_string(),
            "keepass.exe".into(),
            "Bitwarden.exe".into(),
        ];
        history.set_excluded(&programs).unwrap();
        let names: Vec<&str> = history.excluded.iter().map(ProgramName::as_str).collect();
        assert_eq!(names, ["keepass.exe", "bitwarden.exe"]);
        assert!(history.set_excluded(&["a/b".into()]).is_err());
        assert_eq!(history.excluded.len(), 2);
        history.add("kept", None, 1);
        assert_eq!(history.clear(), 1);
        assert_eq!(history.capture, CaptureState::Paused);
        assert_eq!(history.excluded.len(), 2);
        history.set_excluded(&[]).unwrap();
        history.capture = CaptureState::Off;
        assert!(history.is_empty());
    }

    #[test]
    fn the_file_is_typed_versioned_and_lowercases_excluded_programs() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(FILE),
            r#"{ "version": 1, "packages": { "local:/x": {
                "capture": "on", "excluded": ["KeePass.EXE", "1Password"],
                "items": [{ "id": 3, "text": "hi", "copiedAt": 5 }], "nextId": 4 } } }"#,
        )
        .unwrap();
        let store = HistoryStore::open(dir.path());
        let history = store.get("local:/x").unwrap();
        assert_eq!(history.capture, CaptureState::On);
        let names: Vec<&str> = history.excluded.iter().map(ProgramName::as_str).collect();
        assert_eq!(names, ["keepass.exe", "1password"]);
        assert_eq!(history.items[0].text, "hi");

        fs::write(dir.path().join(FILE), r#"{ "version": 2, "packages": {} }"#).unwrap();
        let store = HistoryStore::open(dir.path());
        assert!(store.get("local:/x").unwrap_err().contains("version 2"));
        // Nothing overwrites a file of another version.
        assert!(store.update("local:/x", |_| Ok(())).is_err());
        assert!(
            fs::read_to_string(dir.path().join(FILE))
                .unwrap()
                .contains("\"version\": 2")
        );
    }

    #[test]
    fn a_capture_begun_before_items_were_deleted_keeps_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = HistoryStore::open(dir.path());
        store
            .update("a", |history| {
                history.add("old", None, 1);
                Ok(())
            })
            .unwrap();
        let before = store.deletions();
        store.update("a", |history| Ok(history.clear())).unwrap();
        let add = |packages: &mut BTreeMap<String, PackageHistory>| {
            packages.entry("a".into()).or_default().add("late", None, 2);
            true
        };
        store.capture(before, add);
        assert!(store.get("a").unwrap().items.is_empty());
        store.capture(store.deletions(), add);
        assert_eq!(store.get("a").unwrap().items.len(), 1);
        let on_disk = fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(on_disk.contains("late") && !on_disk.contains("old"));
    }
}
