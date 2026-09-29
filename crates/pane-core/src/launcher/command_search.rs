//! Searching inside an opened command: a command whose manifest entry sets
//! `"search": true`, such as one searching an online service, gets a search
//! field of its own once the user opens it. Pane sends it the text typed
//! there and lists what it finds; root search never asks it, so what the
//! user types there reaches no such command or its service.
//!
//! Each change of the text starts a new search and stops the one before, in
//! the runtime (where it waits, with its web requests) as well as here: an
//! answer to an older text is never shown, whether it arrives late or is
//! the older search's error. The runtime waits a moment
//! ([`crate::runtime::SEARCH_DEBOUNCE`]) before it starts a search, so
//! typing on stops each one before it has asked anything. Leaving the
//! command stops its search too. An error the command answers with, such
//! as a service that is down, is shown in place of results, and counts
//! against the extension no more than any error it answers with: it is not
//! paused for it.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use super::{CommandList, Entry, Launcher, Row, Screen, State, Status, stopped};
use crate::extension_data::PackageData;
use crate::runtime::{CallError, SearchResult, StopCall, View};

/// The search of an open command that searches as the user types.
pub(super) struct Searching {
    /// The command's manifest id, sent with each search.
    command: String,
    /// The command's own list as it last answered it, shown at once while
    /// its search field is blank; `None` until the text is first set.
    list: Option<CommandList>,
    /// Stops the search in progress, if one is; dropping it stops it.
    in_progress: Option<StopCall>,
}

impl Searching {
    pub(super) fn new(command: String) -> Searching {
        Searching {
            command,
            list: None,
            in_progress: None,
        }
    }
}

/// A search started, to be awaited for its answer to be shown.
type Pending = Pin<Box<dyn Future<Output = ()> + Send>>;

impl Launcher {
    /// Sets the text of the open command's search field to `query`: the
    /// search in progress, if any, is stopped. A blank text shows the
    /// command's own list again, as it was last listed, then asks the
    /// command for it anew (its items may have changed since, such as a
    /// setting it shows), which the returned future lists; any other text
    /// asks the command to search, whose answer the returned future shows
    /// (the rows listed meanwhile stay, with a running status). `None` when
    /// nothing is asked.
    pub(super) fn search_in_command(&self, state: &mut State, query: &str) -> Option<Pending> {
        let blank = query.trim().is_empty();
        let command = self.set_search_text(state, query)?;
        let component = state.open.clone()?;
        // The package's generation as of now: disabling or reloading it
        // stops the search.
        let data = self.data_in(state, &component);
        let runtime = match self.runtime() {
            Ok(runtime) => runtime.clone(),
            Err(error) => {
                // Nothing listed is an answer to the text typed.
                state.view.rows.clear();
                state.entries.clear();
                state.view.selected = None;
                state.view.status = Status::Error(error.to_string());
                return None;
            }
        };
        state.view.status = Status::Running;
        let epoch = state.screen_epoch;
        let search = state.search_epoch;
        let launcher = self.clone();
        if blank {
            return Some(Box::pin(async move {
                let answer = runtime.get_view_with(&component, data.clone()).await;
                launcher.show_listed_again(epoch, search, component, data, answer);
            }));
        }
        let (stop, answer) = runtime.search_with(&component, &command, query.trim(), data.clone());
        if let Some(searching) = state.searching.as_mut() {
            searching.in_progress = Some(stop);
        }
        Some(Box::pin(async move {
            let answer = answer.await;
            launcher.show_search_results(epoch, search, component, data, answer);
        }))
    }

    /// Clears the open command's search field (Escape): the search in
    /// progress, if any, is stopped, and the command's own list is shown as
    /// it was last listed, without asking the command.
    pub(super) fn clear_search_in_command(&self, state: &mut State) {
        let _ = self.set_search_text(state, "");
    }

    /// Sets the open command's search text to `query`, stopping the search
    /// in progress; a blank text shows the command's own list as it was
    /// last listed. The command's manifest id; `None` if no open command
    /// searches.
    fn set_search_text(&self, state: &mut State, query: &str) -> Option<String> {
        let searching = state.searching.as_mut()?;
        // Stopped at once: its answer, if it still comes, is not shown.
        searching.in_progress = None;
        state.search_epoch += 1;
        if matches!(&state.view.screen, Screen::CommandSearch { query } if query.trim().is_empty())
        {
            // Leaving the command's own list: kept as it is on screen.
            searching.list = Some(CommandList {
                rows: state.view.rows.clone(),
                entries: state.entries.clone(),
            });
        }
        let kept = if query.trim().is_empty() {
            searching.list.clone()
        } else {
            None
        };
        let command = searching.command.clone();
        state.view.screen = Screen::CommandSearch {
            query: query.to_owned(),
        };
        if let Some(list) = kept {
            self.show_command_list(state, list, Status::Idle);
        }
        Some(command)
    }

    /// Shows `list`, the open command's own list, with `status`.
    fn show_command_list(&self, state: &mut State, list: CommandList, status: Status) {
        state.view.selected = super::first_index(&list.rows);
        state.view.rows = list.rows;
        state.entries = list.entries;
        state.view.status = status;
    }

    /// Lists the open command's `answer` when asked for its own list again
    /// by the search `search`, unless the screen or its text has changed
    /// since. A failure keeps the list as it was, with the error.
    fn show_listed_again(
        &self,
        epoch: u64,
        search: u64,
        component: PathBuf,
        data: Option<PackageData>,
        answer: Result<View, CallError>,
    ) {
        let Some(mut state) = self.lock_if_current(epoch) else {
            return;
        };
        if state.search_epoch != search {
            return;
        }
        let state = &mut *state;
        match (stopped(state, &component, &data), answer) {
            (Some(problem), _) => {
                state.view.rows.clear();
                state.entries.clear();
                state.view.selected = None;
                state.view.status = Status::Error(problem);
            }
            (None, Ok(view)) => {
                let list = CommandList::of(view.items);
                if let Some(searching) = state.searching.as_mut() {
                    searching.list = Some(list.clone());
                }
                self.show_command_list(state, list, Status::Idle);
            }
            (None, Err(error)) => state.view.status = Status::Error(error.to_string()),
        }
    }

    /// Lists the open command's `answer` to the search `search`, unless the
    /// screen or its text has changed since.
    fn show_search_results(
        &self,
        epoch: u64,
        search: u64,
        component: PathBuf,
        data: Option<PackageData>,
        answer: Result<Vec<SearchResult>, CallError>,
    ) {
        let Some(mut state) = self.lock_if_current(epoch) else {
            return;
        };
        if state.search_epoch != search {
            return;
        }
        let state = &mut *state;
        if let Some(searching) = state.searching.as_mut() {
            searching.in_progress = None;
        }
        let (list, status) = match (stopped(state, &component, &data), answer) {
            // Stopped while it was running: its answer is not shown.
            (Some(problem), _) => (CommandList::default(), Status::Error(problem)),
            // Replaced by a newer search, whose answer is shown instead.
            (None, Err(CallError::Cancelled)) => return,
            (None, Ok(results)) => {
                let (rows, entries) = results
                    .into_iter()
                    .map(|result| {
                        let entry = Entry::Run(result.id.clone());
                        (Row::listed(result, None), entry)
                    })
                    .unzip();
                (CommandList { rows, entries }, Status::Idle)
            }
            (None, Err(error)) => (CommandList::default(), Status::Error(error.to_string())),
        };
        self.show_command_list(state, list, status);
    }
}
