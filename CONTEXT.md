# Pane

A general-purpose launcher for Windows, macOS and Linux whose users can install and create extensions around a small core. Pane is the user-selected product name; current prototype testing covers Windows only.

## Language

**Launcher**:
The desktop application through which a user finds and invokes actions.
_Avoid_: Agent, operating system

**Core**:
The essential part of the launcher that remains available independently of installed extensions.
_Avoid_: All bundled features

**Extension**:
An installable addition that contributes functionality to the launcher.
_Avoid_: Plugin, add-on

**Default extension**:
An extension provided by default to supply an everyday feature; the user can disable it individually. It may be acquired automatically during initial setup rather than shipped inside the installer.
_Avoid_: Mandatory feature, core feature

**Disabled extension**:
An installed extension whose execution and contributed functionality are switched off, while its settings and unexpired saved data are retained.
_Avoid_: Uninstalled extension

**Extension settings**:
Values an installed package's commands save through Pane, owned by its package identity and kept while it is disabled, updated or Pane is stopped.
_Avoid_: Preferences, cache

**Extension data**:
Values an installed package's commands keep through Pane, of four kinds (settings, content, cache and local credentials), owned by its package identity; the kind decides what a management action such as clearing its cache removes. Pane removes them itself, never by running the extension.
_Avoid_: Storage, state

**Extension content**:
An extension's own durable records, such as notes or history: extension data kept when its cache is cleared.
_Avoid_: Documents (the user's external files), cache

**Extension cache**:
Extension data the extension can compute or download again, which the user can clear at any time without affecting its settings, content or credentials. Distinct from Pane's own compile cache of components.
_Avoid_: Temporary files, managed copy

**Saved data**:
An extension's settings and content, and the clipboard history Pane keeps for it: the extension data a user chooses to keep or delete when uninstalling it. Its cache and local credentials are removed either way.
_Avoid_: All extension data, durable data (in UI text)

**Uninstall**:
Removing an installed package's managed copy, cache and local credentials, and its saved data if the user chooses, without running it; its source folder and files it saved elsewhere are kept.
_Avoid_: Disable, delete source

**Retained data**:
Extension data Pane keeps for a package identity that is not installed, recorded with the title it had; installing the same source again makes it that package's data again. Manage extensions lists it per identity, and the user can delete it there without the extension, which Pane does itself.
_Avoid_: Orphaned data, leftovers (a leftover is a managed folder awaiting removal)

**Local credential**:
A secret an extension keeps on this computer through Pane, such as a sign-in token. Deleting it does not revoke a remote session.
_Avoid_: Account, session

**Root search**:
The launcher's main search and result view before a specific command is opened.
_Avoid_: Every integration's internal search

**Root result**:
One entry root search lists for a query and can invoke, such as an extension command; it is matched by its title, subtitle and, for an installed command, its package's title, and ranked by the core.
_Avoid_: Item (an item belongs to a command's own list), search hit

**Computed result**:
A root result an extension command computes from the query itself, such as the calculator's answer to "6*7", rather than one found by matching titles; it is listed above those (a file result below them), and invoking it performs its action, such as copying the answer. The search it answers owns the call asking for it: a newer query, or leaving root search, cancels a call still pending.
_Avoid_: Suggestion, answer card, inline result

**Granted folder**:
The one folder the user grants a package through Pane's own "Choose folder…" row, which Pane records itself (not as extension data) and lists for that package's commands under the scan policy; they name its files only by the ids Pane gave them, and Pane opens one after checking it again. The Files default extension's root results come from it.
_Avoid_: Search scope, library, preopen, the extension's folder setting

**Scan policy**:
Pane's fixed bounds on listing a granted folder, the same on every system: regular files only, breadth first in name order, at most 8 folders deep, 5,000 files and 20,000 entries, skipping hidden entries, links and unreadable subfolders; a listing that reaches a bound or skips a subfolder says it is partial. A listing is kept for one visit of root search.
_Avoid_: Indexer, crawl, whole-disk search

**Indexed result**:
A root result an extension command supplies ahead of the query, such as an installed application; Pane asks for them once root search is used, keeps them, and matches and ranks them by title like commands, for a query that is not blank.
_Avoid_: Index entry, cached result

**Installed application**:
A program the operating system lists as installed where Pane looks for it (Start menu shortcuts, application bundles, desktop entries); Pane's host finds and opens it for an extension, which a WASI guest cannot do itself.
_Avoid_: App (ambiguous with Pane itself), program

**Quicklink**:
A named web address the user saves through the Quicklinks default extension's form and finds in root search, where invoking it opens the address with the system's handler for web links; it is kept in that extension's content.
_Avoid_: Bookmark, shortcut, alias

**Clipboard history**:
The text a user copies, which Pane keeps on this computer for an installed package once the user turned it on in the package's command, watching the clipboard only while the history is on and the package runs; a copy its application marks as not to be kept (as password managers do), or from a program the user excluded, is not kept. It is that package's extension data of a kind of its own, written by Pane, never sent anywhere; the Clipboard History default extension shows it.
_Avoid_: Clipboard (the system's current contents, which deleting history never changes), clipboard log, paste history

**Global hotkey**:
A key combination the user assigns to an installed command in Pane, which opens that command in Pane's window while any application has focus; Pane keeps it as its own record and registers it with the system only while the command's extension is enabled.
_Avoid_: Shortcut (any key combination, including Pane's own keys), keybinding, alias

**Alias**:
A word the user gives an installed command in Pane; typing it in root search lists that command first, and, for a query-taking command, typing it before some text lists a row that sends the text to the command when invoked. Pane keeps it as its own record by command id; a disabled package's commands offer none.
_Avoid_: Keyword (an author's search term), shortcut, nickname

**Fallback**:
A query-taking command the user chose to have offered below root search's results for any text typed; it is never chosen by itself, so the text reaches it only when the user invokes it.
_Avoid_: Default action, catch-all

**Query-taking command**:
An extension command that declares it takes a query: text typed in root search, which Pane sends it only when the user invokes it through its alias or as a fallback, and whose answer Pane shows.
_Avoid_: Argument (Raycast's per-field input), search provider (a provider is asked while the user types)

**Command search**:
The search field an opened command has when it searches as the user types, such as one searching an online service; Pane sends the text typed there only to that command, stops a search the text has replaced, and never asks the command from root search.
_Avoid_: Search provider (root search asks those), query-taking command (sent text from root search once, when invoked)

**Network use**:
What Pane shows of an installed package's web requests, which it does not gate: whether its component imports `wasi:http`, and the addresses it tried to reach since Pane started.
_Avoid_: Network permission (nothing is granted or refused)

**Search provider**:
A source of matching results for a query, such as applications, files or an online service.
_Avoid_: The entire search interface

**Package identity**:
The identity that distinguishes an installed source package from other packages, independently of its display title or selected release.
_Avoid_: Display name, command name

**Extension package**:
A unit of installation: a package manifest plus the built components of the commands it lists. A local package is a folder.
_Avoid_: Plugin bundle

**Package manifest**:
The `pane.json` file that declares a package's title, version, required extension API, commands, operations and dependencies, versioned by its manifest version.
_Avoid_: package.json (npm's file)

**Source-only package**:
A package whose manifest names components that have not been built, in a folder or published to npm without them; Pane explains it rather than installing it, and never builds it.
_Avoid_: Broken install

**npm-distributed package**:
An extension package published to the npm registry: a tarball holding its package manifest and built components, identified by its npm name without version. Pane downloads it itself, checks its integrity and unpacks only its files and folders; the unpacked package is then installed like a local package, into a managed copy, while it keeps its npm source identity. Pane runs none of its npm install scripts and installs none of its npm dependencies.
_Avoid_: Node package, npm module (Pane runs no Node code), plugin from npm

**Pinned version**:
The exact npm version the user, or a dependency's source, named when a package from npm was installed or updated, recorded so that it is not taken for the latest; updating without a version keeps it, and naming another version changes it. A dependency's source that names a version must get that version: an installed copy of another version is a conflict, since installing another package never replaces an installed required dependency.
_Avoid_: Locked version, version range

**Managed copy**:
Pane's own copy of an installed package's manifest and components, kept in Pane's data folder, separate from the user-owned source.
_Avoid_: Cache (it is not disposable)

**Update**:
Replacing the managed copy of an installed package from its source while keeping its package identity. A second explicit install of the same identity is rejected instead.
_Avoid_: Reinstall

**Reload**:
Replacing an installed package's code from its source folder while Pane and other packages keep running: the replacement is checked as an install would check it, then replaces the managed copy, the old instances stop and the new code starts. Settings are kept; live state is not carried over.
_Avoid_: Restart, hot swap, update (an update does not start the new code)

**Development mode**:
An installed local package whose source folder Pane watches while its author works on it: each save runs the package's documented build command in that folder, staging the components under Pane's data folder, and a build that succeeds reloads the package from there, while one that fails keeps its working code and shows the build's diagnostics. It lasts until the author stops it, the package is disabled or uninstalled, or Pane quits, each of which kills a running build with the processes it started; another installed copy of the package is never affected.
_Avoid_: Watch mode, hot reload, dev copy (a copy is an installation)

**Build failure**:
A development build that did not succeed: nothing is replaced, and the package keeps running its installed code. Distinct from a startup failure, whose replacement was installed.
_Avoid_: Crash, startup failure

**Obsolete build**:
A development build during which the source was saved again: it is never reloaded, what it left in the source folder is replaced with the installed components, and the folder is built again, so an older build cannot replace a newer one; after three in a row, Pane waits for the next save.
_Avoid_: Cancelled build (it runs to its end)

**Startup failure**:
A reload whose checked replacement was installed but could not start: a command trapped, or its component could not load or be instantiated (an error the command returns for its view is not one); Pane pauses the package, reporting it with Retry and diagnostics, and does not restore the earlier code. Distinct from a replacement that fails its checks, or a build failure, which leave the working code in place.
_Avoid_: Build failure, rollback

**Paused extension**:
An enabled extension Pane stopped running after a failure attributable to it: it could not start, or it crashed three times within five minutes (an error it answers with is not a failure, nor a call stopped because a generation ended). Its commands stay listed, saying why they do not run; its saved data is kept, and the pause holds across restarts until the user retries, reloads, updates, disables or enables it. Distinct from a disabled extension, which is the user's choice.
_Avoid_: Crashed extension, quarantined, disabled (by Pane)

**Extension runtime**:
The part of Pane that runs every installed extension's code (the Wasmtime engine; the runtime is a thread in Pane's process today), shared by all extensions; the window, root search's own rows and Manage extensions do not depend on it.
_Avoid_: Engine (one part of it)

**Runtime crash**:
A failure of the extension runtime itself, not attributable to any one extension, such as a panic of its thread: every call it held is stopped and none is run again by itself, even if its effect was done and only its answer lost; Pane names and pauses no extension, keeps saved data, ends the native helpers it ran and starts the runtime again, unless it crashed within five minutes before, when it stays stopped until the user restarts it in Manage extensions. Distinct from an extension's crash (a guest trap), which counts towards pausing that extension.
_Avoid_: Extension crash, paused runtime

**Generation**:
One run of an installed package's code, from when it is installed, enabled or Pane starts until it is disabled, paused or its code is replaced by a reload or an update. Every call into the package belongs to the generation current when it was asked for, and is stopped when that generation ends; its late result is discarded.
_Avoid_: Version (a package's version is its manifest's), session, instance (one generation can start several), screen or search epoch (the launcher's counters of screens and searches, which decide whether an answer is shown; a search also cancels its own pending calls for computed results)

**Background work**:
Extension code Pane runs without the user opening anything, in an instance of its own beside the package's calls: it belongs to its package's generation, stops when that generation ends (its late result discarded) and holds back no call while it waits.
_Avoid_: Daemon, worker (a JavaScript worker is the Node helper's), background call

**Scheduled task**:
A command's background work its package manifest declares with a schedule (every so many minutes, from 1 to 1440): Pane runs it only after the user turns its schedule on in Manage extensions, at once and then once per interval while the package may run, one run at a time, and shows its latest result there; what fell due while it could not run runs once.
_Avoid_: Cron job, timer, periodic command

**Continuing service**:
A command's background work its package manifest declares with `"service": true`, which Pane keeps running once the user starts it in Manage extensions: whenever its package's code runs (starting again with Pane and with each new generation), one run at a time, showing the status it sets; an error or a crash starts it again a minute later, and a crash counts towards pausing its package. Unused, it never runs.
_Avoid_: Daemon, background process (it is not a process), long-running command

**Supported platforms**:
The operating systems a package, a command or an action declares it works on: a plain list, not a rule language. A declaration is not evidence of native support.
_Avoid_: Compatibility rules, target matrix

**Unavailable action**:
An action whose supported platforms exclude the current system; Pane keeps it listed, explains why and never runs it, so the extension's other actions stay usable.
_Avoid_: Hidden action, disabled extension

**Operation**:
A named, versioned function an installed package publishes in its package manifest for other extensions to call through Pane, with JSON input and result; only published operations are callable, so a command is never one implicitly.
_Avoid_: API, command (a command is what the user opens), endpoint

**Dependency**:
Another package whose operations a package calls, declared in its package manifest with the source it comes from and the operations and versions it calls; the package's code calls it by the declaration's id.
_Avoid_: Library dependency (an npm or Cargo library bundled into a component), extension pack

**Required dependency**:
A dependency a package needs: installing the package shows it and installs it first if it is missing, but never replaces an installed copy (which counts as pinned) or enables a disabled one; a required dependency that cannot be installed or does not publish what is called stops the install before anything changes.
_Avoid_: Hard dependency, prerequisite

**Required dependent**:
An installed package that requires another on this system, directly or through other installed packages that do (its required dependent closure; optional dependencies never count). Disabling the package it requires first shows the enabled ones, which are disabled together or not at all (Disable all or Cancel); enabling that package again does not enable them. Uninstalling it first shows all of them, disabled ones too, with their saved data; they are uninstalled together, keeping or deleting their saved data, or not at all (Uninstall all or Cancel), and installing that package again does not install them.
_Avoid_: Reverse dependency, child extension

**Optional dependency**:
A dependency a package uses only when the user installed it; installing the package lists it but never installs it.
_Avoid_: Soft dependency, suggestion, recommended extension

**Call chain**:
The operation calls waiting on one another at one moment, from the command that made the first; each package in it is busy until its call returns, so a call back into one is refused rather than waited on.
_Avoid_: Call stack (of one guest), workflow

**Native helper**:
A prebuilt program an installed package ships for each target (operating system and processor) it supports, which its commands run through Pane for what a WASI guest cannot do; Pane runs this system's file, never compiles one, and ends its process when the command cancels the run, the call that started it returns, the package's generation ends or Pane quits. Processes the helper starts itself are its own.
_Avoid_: Plugin binary, native extension (the extension's entry point stays a WASI component), sidecar

**Helper target**:
The operating system and processor a native helper's file is built for, written `<os>-<arch>` in the package manifest, such as `linux-x86_64` or `macos-aarch64`; Pane runs only the file for its own target.
_Avoid_: Platform (a supported platform is an operating system alone), triple

**Form**:
A set of fields an extension command asks the user to fill in and submit; the launcher renders its standard controls and the extension validates the submitted values.
_Avoid_: Dialog, custom view

**Custom view**:
An interactive view an extension draws itself from shapes the launcher paints, receiving the user's key and pointer input while it is open; the launcher keeps focus and its accessible representation.
_Avoid_: Canvas, webview, custom control
