//! Disabling or uninstalling a package that other installed packages
//! require.
//!
//! When the user disables a package from the extension list and enabled
//! packages require it, directly or through each other (the required
//! dependent closure of `dependencies::required_dependents`, which leaves
//! out optional dependencies and those needed only on other systems),
//! nothing changes yet: a confirmation lists them, with Disable all and
//! Cancel. Cancel (or Back) returns to the list with everything as it was.
//! Disable all disables the package and exactly the dependents shown, in one
//! record, as an ordinary disable of each: their commands leave root search,
//! their instances stop, and their settings and data are kept. If an
//! enabled dependent that was not shown has appeared meanwhile, nothing is
//! disabled and the new set is shown instead.
//!
//! Enabling a package again enables it alone. Pane pausing a package after
//! it failed is not this user action and disables nothing else.
//!
//! Uninstalling a package that installed packages require, enabled or
//! disabled, asks the same way, with each one's saved data and the choice
//! of a single uninstall: Uninstall all keeping saved data, Uninstall all
//! deleting it, or Cancel. Uninstall all uninstalls exactly the shown set,
//! as uninstalling each would, recorded in one write. Installing the
//! package again installs it alone.

use super::uninstall::Uninstall;
use super::{Change, Entry, Launcher, LauncherView, Question, Row, Screen, State, Status};
use crate::dependencies::{self, Dependent};
use crate::extension_data::DataKind;
use crate::packages::{PackageError, PackageIdentity, SavedData};
use crate::platform;

/// "Disabled A and B, which requires it", after disabling `title` and the
/// packages titled `dependents` that require it.
pub(super) fn disabled(title: &str, dependents: &[String]) -> String {
    format!("Disabled {}", with_dependents(title, dependents))
}

/// "A and B, which requires it" or "A and the 2 extensions that require it:
/// B and C": the package titled `title` and the packages titled
/// `dependents` that require it.
pub(super) fn with_dependents(title: &str, dependents: &[String]) -> String {
    match dependents {
        [dependent] => format!("{title} and {dependent}, which requires it"),
        _ => format!(
            "{title} and the {} extensions that require it: {}",
            dependents.len(),
            platform::join(dependents)
        ),
    }
}

impl Launcher {
    /// Asks whether to disable the installed package with `identity`
    /// together with the enabled packages of `closure`, its required
    /// dependents, listing them, and those of them disabled already.
    pub(super) fn show_disable_dependents(
        &self,
        state: &mut State,
        identity: &PackageIdentity,
        closure: Vec<Dependent>,
    ) {
        let title = state.title_of(identity);
        let (to_disable, already): (Vec<Dependent>, Vec<Dependent>) =
            closure.into_iter().partition(|dependent| dependent.enabled);
        let shown: Vec<PackageIdentity> = to_disable
            .iter()
            .map(|dependent| dependent.package.identity.clone())
            .collect();
        let mut details = vec![
            format!("From {identity}"),
            format!(
                "These extensions require {title}, directly or through each other, and cannot \
                 work without it, so they are disabled with it:"
            ),
        ];
        details.extend(to_disable.iter().map(|dependent| {
            format!(
                "{}, which requires {} · {}",
                dependent.package.title, dependent.requires.title, dependent.package.identity
            )
        }));
        details.extend(already.iter().map(|dependent| {
            format!(
                "Already disabled: {}, which requires {}",
                dependent.package.title, dependent.requires.title
            )
        }));
        details.push(format!(
            "Each keeps its settings and saved data. Enabling {title} again does not enable \
             them: enable each in Manage extensions."
        ));
        let with = match to_disable.as_slice() {
            [dependent] => format!("{}, which requires it", dependent.package.title),
            _ => format!("the {} extensions that require it", to_disable.len()),
        };
        let choice = |id: &str, title: String, subtitle: String| Row {
            id: id.into(),
            title,
            subtitle: Some(subtitle),
            unavailable: None,
        };
        let rows = vec![
            choice(
                "disable-all",
                format!("Disable all {}", to_disable.len() + 1),
                format!("Disable {title} and {with}"),
            ),
            choice("cancel", "Cancel".into(), "Keep them all enabled".into()),
        ];
        state.next_screen();
        state.entries = vec![Entry::DisableAll(identity.clone(), shown), Entry::Cancel];
        let screen = Screen::Confirm {
            question: Question::DisableDependents(identity.clone()),
            details,
        };
        state.view = LauncherView::new(
            screen,
            format!("Disable {title} and the extensions that require it?"),
        )
        .with_rows(rows);
    }

    /// Disables the package with `identity` and the enabled packages that
    /// require it, if they are among those the confirmation showed
    /// (`shown`), to be recorded by `finish_change`, and shows the
    /// extension list. If the package was disabled meanwhile, disables
    /// nothing; else if one that was not shown requires it now, disables
    /// nothing and asks again.
    pub(super) fn begin_disable_all(
        &self,
        state: &mut State,
        identity: PackageIdentity,
        shown: &[PackageIdentity],
    ) -> Option<Change> {
        let Some(package) = state.package(&identity) else {
            self.show_extensions(state);
            state.view.status = Status::Error(PackageError::NotInstalled(identity).to_string());
            return None;
        };
        let title = package.title();
        if !package.enabled {
            // Disabled meanwhile, which asks nothing more of its dependents,
            // even if a new one appeared.
            self.show_extensions_at(
                state,
                |entry| matches!(entry, Entry::Toggle(asked) if *asked == identity),
            );
            state.view.status = Status::Error(format!("{title} is disabled already"));
            return None;
        }
        let identities = self.still_shown(state, &identity, &title, shown, Together::Disable)?;
        self.show_extensions_at(
            state,
            |entry| matches!(entry, Entry::Toggle(asked) if *asked == identity),
        );
        self.begin_change(state, identities, false)
    }

    /// Asks whether to uninstall the installed package with `identity`
    /// together with the packages of `closure`, its required dependents
    /// (disabled ones included), listing them with each one's saved data,
    /// with a choice to keep or delete that data.
    pub(super) fn show_uninstall_dependents(
        &self,
        state: &mut State,
        identity: &PackageIdentity,
        closure: Vec<Dependent>,
    ) {
        let Some(installation) = &self.installation else {
            return;
        };
        let title = state.title_of(identity);
        let shown: Vec<PackageIdentity> = closure
            .iter()
            .map(|dependent| dependent.package.identity.clone())
            .collect();
        let mut details = vec![
            format!("From {identity}"),
            format!(
                "These extensions require {title}, directly or through each other, so they are \
                 uninstalled with it; installing {title} again does not install them again:"
            ),
        ];
        details.extend(closure.iter().map(|dependent| {
            let disabled = if dependent.enabled { "" } else { " (disabled)" };
            format!(
                "{}{disabled}, which requires {} · {}",
                dependent.package.title, dependent.requires.title, dependent.package.identity
            )
        }));
        let kept = installation.data.kept_now(&DataKind::SAVED);
        let named = std::iter::once((identity, title.clone())).chain(
            closure
                .iter()
                .map(|dependent| (&dependent.package.identity, dependent.package.title.clone())),
        );
        let saved: Vec<String> = named
            .map(|(identity, title)| {
                let saved = kept.describe(identity).unwrap_or_else(|| "none".into());
                format!("{title} {saved}")
            })
            .collect();
        details.push(format!("Saved data: {}", saved.join(" · ")));
        details.push(
            "Pane removes the installed copy, cache and credentials of each, without running \
             it; their source folders and files they saved elsewhere are not touched. Deleting \
             a credential does not sign you out of an online service."
                .into(),
        );
        let count = closure.len() + 1;
        let choice = |id: &str, title: String, subtitle: String| Row {
            id: id.into(),
            title,
            subtitle: Some(subtitle),
            unavailable: None,
        };
        let rows = vec![
            choice(
                "keep",
                format!("Uninstall all {count} and keep saved data"),
                "Keep each one's settings and content; installing one again from the same \
                 source restores its own"
                    .into(),
            ),
            choice(
                "delete",
                format!("Uninstall all {count} and delete saved data"),
                format!("Delete the settings and content of all {count} too"),
            ),
            choice("cancel", "Cancel".into(), "Keep them all installed".into()),
        ];
        state.screen_epoch += 1;
        state.entries = vec![
            Entry::UninstallAll(identity.clone(), shown.clone(), SavedData::Keep),
            Entry::UninstallAll(identity.clone(), shown, SavedData::Delete),
            Entry::Cancel,
        ];
        let screen = Screen::Confirm {
            question: Question::UninstallDependents(identity.clone()),
            details,
        };
        state.view = LauncherView::new(
            screen,
            format!("Uninstall {title} and the extensions that require it?"),
        )
        .with_rows(rows);
    }

    /// Removes the package with `identity` and the installed packages that
    /// require it from this launcher, to be uninstalled together by
    /// `finish_uninstall`, if they are among those the confirmation showed
    /// (`shown`); one of those uninstalled meanwhile is skipped. If the
    /// package is not installed any more, removes nothing; else if one that
    /// was not shown requires it now, removes nothing and asks again.
    pub(super) fn begin_uninstall_all(
        &self,
        state: &mut State,
        identity: PackageIdentity,
        shown: &[PackageIdentity],
        saved: SavedData,
    ) -> Option<Uninstall> {
        let Some(package) = state.package(&identity) else {
            self.show_extensions(state);
            state.view.status = Status::Error(PackageError::NotInstalled(identity).to_string());
            return None;
        };
        let title = package.title();
        let identities = self.still_shown(state, &identity, &title, shown, Together::Uninstall)?;
        self.begin_uninstall(state, identities, saved)
    }

    /// The package with `identity`, titled `title`, and its required
    /// dependents that `together` changes, as they are now, the package
    /// first: those the confirmation showed (`shown`), less any that left
    /// the closure meanwhile. If one that was not shown is in it now, shows
    /// the question again with the new set, says why, and returns `None`.
    fn still_shown(
        &self,
        state: &mut State,
        identity: &PackageIdentity,
        title: &str,
        shown: &[PackageIdentity],
        together: Together,
    ) -> Option<Vec<PackageIdentity>> {
        let closure = dependencies::required_dependents(&state.packages, identity);
        let now: Vec<PackageIdentity> = closure
            .iter()
            .filter(|dependent| together.changes(dependent))
            .map(|dependent| dependent.package.identity.clone())
            .collect();
        if now.iter().any(|dependent| !shown.contains(dependent)) {
            let (doing, choice) = match together {
                Together::Disable => {
                    self.show_disable_dependents(state, identity, closure);
                    ("disabling", "Disable all")
                }
                Together::Uninstall => {
                    self.show_uninstall_dependents(state, identity, closure);
                    ("uninstalling", "Uninstall all")
                }
            };
            state.view.status = Status::Error(format!(
                "What {doing} {title} affects changed since it was shown; check it again and \
                 choose {choice} once more"
            ));
            return None;
        }
        Some(std::iter::once(identity.clone()).chain(now).collect())
    }
}

/// What a confirmation does to a package together with its required
/// dependents.
#[derive(Clone, Copy)]
enum Together {
    /// Disable All: the enabled dependents.
    Disable,
    /// Uninstall All: every dependent, disabled ones too.
    Uninstall,
}

impl Together {
    /// Whether it changes `dependent`.
    fn changes(self, dependent: &Dependent) -> bool {
        match self {
            Together::Disable => dependent.enabled,
            Together::Uninstall => true,
        }
    }
}
