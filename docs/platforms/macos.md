# macOS native baseline (#7)

Recorded 2026-09-28 from GitHub Actions run
[36366760796](https://github.com/wasimysaid/pane/actions/runs/36366760796) on the
fork `wasimysaid/pane`, commit `572d629`.

## Tested combination

| | |
| --- | --- |
| OS | macOS 15.7.9 (build 24G830), from the smoke's `sw_vers` |
| Architecture | arm64 (Apple silicon) |
| Machine | GitHub-hosted runner, image `macos-15-arm64` version 20260907.0337.1 |
| Session | the runner's logged-in GUI session: WindowServer, Finder, Dock, real display surface |
| Rust | 1.98.1 (`rust-toolchain.toml`) |

**Not tested, so not claimed:** Intel (x86_64) Macs, macOS 14 or earlier and 26
or later, a physical Mac with a real display, Retina scaling other than the
runner's, and any signed or notarized build.

## Fresh checkout build and checks

The CI job checks out the repository and runs only the documented commands (the
README's macOS prerequisite is the Xcode Command Line Tools, preinstalled on the
runner):

```sh
rustup toolchain install
cargo xtask ci          # guests, prebuilt JS/TS check, fmt, clippy, all tests
cargo build --locked -p pane
```

Result: `cargo xtask ci` passed (13 window, 9 launcher-model, 1 runtime-cache
and 25 sample-contract tests), and the launcher built. No Windows machine or
PowerShell was involved. The Rust guests are built natively; the JS and TS
samples use the committed prebuilt components. Rebuilding those from source on
macOS (`cargo xtask js-guests`) has **not** been run. The Cargo cache
(`Swatinem/rust-cache`) was warm, so this is a fresh checkout but not a cold
`target/`.

## Native GUI smoke

```sh
cargo build -p pane
scripts/smoke-macos.sh smoke            # needs Python 3 with Pillow
```

The script launches `target/debug/pane`, brings it to the front and sends real
key events through System Events (`osascript`), which needs the Accessibility
permission for the calling terminal (GitHub's macOS runners grant it). For each
of the Rust, JavaScript and TypeScript sample commands it presses Return to open
it, Down and Return to run "Wait briefly" (an async WASI 0.3 clock import inside
the guest), then Escape.

It fails if Pane exits. Three checks also run against the `screencapture`
screenshots (`scripts/check_screenshot.py`):

- The root screen draws text in Pane's hint color.
- Each result screen draws text in the result color.
- The three result screens are all different. This catches a smoke that opens
  the same command twice.

Screenshots, cropped to the window (and inspected):

| Step | Evidence |
| --- | --- |
| Root search lists the three sample commands | [1-root.png](evidence/macos/1-root.png) |
| Rust command opened; action result "Waited 50 ms inside the Rust guest" | [2-command-0.png](evidence/macos/2-command-0.png), [2-result-0.png](evidence/macos/2-result-0.png) |
| JavaScript command; "Waited 50 ms inside the JavaScript guest" | [3-command-1.png](evidence/macos/3-command-1.png), [3-result-1.png](evidence/macos/3-result-1.png) |
| TypeScript command; "Waited 50 ms inside the TypeScript guest" | [4-command-2.png](evidence/macos/4-command-2.png), [4-result-2.png](evidence/macos/4-result-2.png) |
| Escape returns to root search | [5-back-to-root.png](evidence/macos/5-back-to-root.png) |

Rendering, keyboard focus, selection, guest execution and result display all
worked natively.

## macOS-specific fixes made for this sample

- **Text rendering:** GPUI CE's macOS text system is behind the `font-kit`
  feature of `gpui_platform`, which was off. The first run
  ([36363402510](https://github.com/wasimysaid/pane/actions/runs/36363402510))
  drew backgrounds but no text. The feature is now enabled in
  `crates/pane/Cargo.toml`.
- **Smoke script portability:** BSD `seq` prints `1 0` for `seq 0`, so an earlier
  smoke opened the TypeScript command where it meant to open Rust, and passed
  because it only checked that text was drawn. The key loop now counts in bash
  arithmetic, and the smoke asserts that the result screens differ.

No process or path changes were needed. On every OS the launcher reads guests
from `PANE_EXTENSIONS_DIR`, or else from `target/guests` in the source tree it
was built from. That is a development-build assumption, and the installer
tickets will replace it.

## Platform availability (#19)

The smoke also runs the platform-availability steps (screenshots 13 to 15,
[platform availability](../platform-availability.md#checks)): the Rust
command's Windows-only and macOS-and-Linux actions, then a package listing
only the two other systems. In run [36372625940](https://github.com/wasimysaid/pane/actions/runs/36372625940) (commit `38a95cb`, macOS 15.7.9, arm64) every step passed: the Windows-only action was listed with "Not available on macOS: this action supports only Windows" and did not run, the macOS-and-Linux action answered, and the package for Windows and Linux was refused with "Not available on macOS: this package supports only Windows and Linux". The list scrolled to keep the selected row visible. The later #19 fixes (per-command platforms, re-focusing before these steps) have not run here yet.

| Step | Evidence |
| --- | --- |
| Windows-only action | [13-windows-only.png](evidence/macos/13-windows-only.png) |
| macOS-and-Linux action | [14-not-windows.png](evidence/macos/14-not-windows.png) |
| Package for the other two systems | [15-no-compatible-package.png](evidence/macos/15-no-compatible-package.png) |

## Root search (#23)

Root search has a query field with focus ([root search](../root-search.md)).
The smoke's search phase (screenshots 24 to 26) types "typescr" with System
Events `keystroke`, opens the only match and runs "Wait briefly", which must
look exactly like step 4, then types "zzz" and presses Return on no results.
In run [36420611977](https://github.com/wasimysaid/pane/actions/runs/36420611977) (commit `6d73d18`) every step passed: typing "typescr" left only TypeScript sample and Enter ran it, and "zzz" showed No results ([24-search.png](evidence/macos/24-search.png), [26-no-results.png](evidence/macos/26-no-results.png)). Input-method composition in the query field is still unverified here.

## Calculator (#27)

The smoke's calculator phase (screenshots 27 to 30) installs the calculator
package, types "6*7", checks the selected answer row, presses Enter to copy
it, then compares typing "42+1" with pasting the copy (Cmd+A, Cmd+V) and typing
"+1", which must look the same. In run [36423871204](https://github.com/wasimysaid/pane/actions/runs/36423871204) (commit `ab91081`) every step passed: "6*7" answered 42, Enter copied it, and pasting then typing "+1" matched typing "42+1", so the system clipboard held "42" ([27-answer.png](evidence/macos/27-answer.png), [30-pasted.png](evidence/macos/30-pasted.png)).

## Applications (#25)

[Applications](../applications.md) finds the `.app` bundles in
`/Applications`, `/System/Applications` and `~/Applications` (and their
subfolders such as `Utilities`) and opens one with `/usr/bin/open`. The
smoke's last phase (screenshots 44 and 45) makes a bundle "Pane Smoke App"
whose program is a shell script writing a marker file, in `~/Applications`
of a HOME given to Pane only, installs the package, types "pane smoke",
checks the selected row, presses Return and checks "Opened Pane Smoke App"
and the marker. The adapter tests also open such a bundle and require
Calculator among the system's applications. **Not run on macOS yet**: this
branch was not pushed, so the phase, the native tests and the
`open`/Launch Services path are unverified here, including whether Launch
Services runs an unsigned script bundle on the runner.

## Quicklinks (#28)

The smoke's last phase (screenshots 46 and 47) installs the Quicklinks
package, creates "Pane issues" (https://example.com/pane-issues) in its
form, restarts Pane and types "pane iss", which must list it selected. It
stops before Enter, which would open the default browser; opening a link
here is checked only through the tests' recording opener. Not run yet.

## Global hotkeys (#33)

The smoke's hotkey phase (screenshots 52 to 58, [global hotkeys](../hotkeys.md#checks))
assigns Control+Option+G to Greeting on its hotkey screen, brings Finder to
the front, presses it through System Events and checks that Pane is the
frontmost process with Greeting open; then again after a restart; then,
with the extension disabled, that pressing it leaves Finder in front and
Pane's window unchanged. The hotkey is a Carbon hot key
(`RegisterEventHotKey`), which needs no Accessibility or Input Monitoring
permission of Pane's own (System Events, which sends the keys, needs the
Accessibility permission the runners grant). The adapter code was only
compile- and lint-checked for `x86_64-apple-darwin` from Linux; **not run
on macOS yet**, so registration, delivery on the main run loop and the
focus transition are unverified natively.

## Deleting retained data (#41)

The smoke's retained-data phase (screenshots 63 to 65, [deleting retained
data](../extension-data.md#deleting-retained-data)), with a data folder of
its own, saves a note with the settings sample, uninstalls it keeping its
saved data, deletes its retained data from the extension list's last row
(Down from the selected Cancel, then Return), checks that `installed.json`
and `content.json` no longer hold it, and reinstalls the same folder, which
must show nothing kept. **Not run on macOS yet.**

## Aliases and fallbacks (#31)

The smoke's alias phase (screenshots 66 to 74, [aliases and fallbacks](../aliases.md#checks))
gives Echo, the query sample's command, the alias "ec" and makes it a
fallback in Manage extensions, sends "ec hello" and, from the unselected
fallback row chosen with Down, "zqx" to it, and checks that with the
extension disabled "ec hello" lists nothing. Nothing in it is specific to
macOS (no system API is involved); **not run on macOS yet**.

## Dependencies (#42)

The dependencies phase (screenshots 75 to 77, [dependencies](../dependencies.md#checks)),
with a data folder of its own, previews the dependencies sample (its
required JavaScript operations sample and optional Rust one listed),
installs it with the JavaScript sample and runs "Greet through the required
greeter", which must answer from the JavaScript guest; `installed.json`
must then hold exactly two packages and the recorded dependency. The logic
is platform-independent except path resolution, which uses the same
`canonicalize` as package identity. **Not run on macOS yet.**

## Native helpers (#15)

The first native-helper phase (screenshots 90 to 93, data folder `helper-data`,
[native helpers](../helpers.md#checks)) installs the helper sample, whose
`pane-echo` `cargo xtask guests` builds for the runner (`macos-aarch64` on
`macos-15`), runs it (the answer must name macOS arm64), cancels a slow run
after a second, starts the ten-second run, checks with `pgrep` that the
helper runs from the managed copy, disables the package and checks that the
process is gone, that the saved "started" note is kept, and that no helper
outlives Pane. A second phase (screenshot 94, `helper-quit-data`) starts
the waiting helper, asks Pane to quit with a quit Apple event
(`NSRunningApplication.terminate`, through Python's ctypes), and checks
that Pane exits, no helper runs and its heartbeat file stops growing. The
tests in `crates/pane-core/tests/helpers.rs` (Rust, JavaScript and
TypeScript samples) and the runner's unit tests (Mach-O headers) run in
`cargo xtask ci` there, against the `pane-echo` built natively on the
runner. The runner was only compile- and lint-checked for
`x86_64-apple-darwin` from Linux; **not run on macOS yet**, so starting,
ending and reaping a helper natively, and the executable permission of the
copied file, are unverified there, and whether GPUI runs Pane's quit
handler for a quit Apple event is unverified until the smoke runs. No macOS x86-64 build was made.

## Development mode (#12, #13)

The smoke's last phase (screenshots 110 to 136, [development
mode](../development-mode.md#checks)) builds a copy of each development
sample, develops it from Manage extensions, saves an edit, a change that
does not build, two saves in a row and, after stopping, one more, checking
the answers, the error and that nothing is built after stopping. The
TypeScript and JavaScript samples run only where the JS toolchain is built,
so CI's smoke runs the Rust one. The platform code (FSEvents through notify, with the folder and event
paths made canonical, and a process group killed with `SIGKILL`) was only
compile- and lint-checked for `x86_64-apple-darwin` from Linux; **not run on
macOS yet**, so the file watcher's events, the build's processes being
killed and the whole phase are unverified natively. macOS has no parent
death signal, so a build outlives a Pane that is killed or crashes (one
that quits kills it).

## Disabling required dependents (#43)

The disable-dependents phase (screenshots 140 to 143, [disabling a required dependency](../dependencies.md#disabling-a-required-dependency)),
with a data folder of its own, installs the dependencies sample, presses
Enter on the JavaScript operations sample in Manage extensions, which must
ask first (the details color), cancels, then chooses Disable all (the
result color) and enables the JavaScript sample again alone (the result
color); `installed.json` must then record exactly one disabled package.
Nothing in it is specific to macOS (no system API is involved; the
closure reuses the dependency identities recorded at install); **not run on
macOS yet**.

## Runtime crashes (#17)

The runtime-crash phase (screenshots 200 to 209, data folder
`runtime-crash-data`, [runtime crashes](../pausing.md#when-the-extension-runtime-itself-crashes))
installs the helper and settings samples, starts Pane with
`PANE_TEST_RUNTIME_FAULTS` naming a fault file, runs the settings sample's
Count, starts the waiting helper and has the runtime crash: the helper must
be gone (`pgrep`, and its heartbeat must stop growing), the note it saved kept,
and the status line the error color. A second crash, injected before
Count's answer, must leave the count at 2 and the runtime stopped; root
search explains it, Manage extensions shows why (the details color), a
disable works, **Restart the extension runtime** runs extensions again and
Count then counts 3; no package may be recorded as paused. Nothing in it is
specific to macOS (the runtime is a thread; helpers are ended as for a
disable); **not run on macOS yet**.

## Uninstalling required dependents (#44)

A phase of the smoke (screenshots 180 to 183, [uninstalling a required dependency](../dependencies.md#uninstalling-a-required-dependency)),
with a data folder of its own, installs the dependencies sample, presses
Enter on "Uninstall JavaScript operations sample" in Manage extensions,
which must ask first (the details color), cancels, then chooses Uninstall
all 2 keeping saved data (the result color); `installed.json` must then hold
no package. Pane is started again to install the JavaScript operations
sample alone, and `installed.json` must then hold exactly one package.
Removing a managed copy uses the same `remove_dir_all` as a single
uninstall; a folder still in use is listed and removed at the next start,
reported against its own package. Nothing else in it is specific to
macOS; **not run on macOS yet**.

## npm packages (#45)

A phase of the smoke (screenshots 260 to 266, [npm packages](../npm.md)),
with a data folder of its own, starts `scripts/npm_registry.py` on
127.0.0.1 serving the npm sample `cargo xtask guests` packed, and points the
development build at it with `PANE_NPM_REGISTRY`: it installs the local
Dependencies from npm sample, which downloads and installs the npm package
it requires, calls its `greet` operation, then names the npm package in
"Install extension from npm…" (Up from the last row), updates it and runs
its command, which answers "Hello from the npm package"; `installed.json`
must then record `"npm": "@pane-samples/greeter"` at `"npmVersion":
"0.1.0"` with both packages. The real registry would be reached through the
HTTP client guests' requests use, trusting the certificates rustls-native-certs
reads from the system keychains, which no check exercises (the smoke never
reaches the network; [by hand](../npm.md#trying-the-real-registry-by-hand),
not run); nothing else in it is specific to macOS.
The packing, in `cargo xtask guests`, runs in CI on macOS. **Not run on
macOS yet.**

## Files (#29)

The files phase (screenshots 220 to 223, [files](../files.md)), with a data
folder of its own, installs Files and presses Return on Pane's own "Choose
folder…" row; a debug build's `PANE_TEST_CHOOSE_FOLDER` names a fixture
folder "Pane smoke files" (spaces) in the system's temporary folder instead
of showing the system's picker. It types "plan", which must list "Résumé
plan ü.txt" selected, and presses Return. A debug build's
`PANE_TEST_OPEN_FILE_LOG` makes the opener record the path in a file instead
of running `/usr/bin/open`, so no application of the user's opens it; the
recorded path, resolved, must be the fixture file's, resolved. Last it types
"runner" and presses Return on an executable script, which Pane must refuse
without recording or running it. A positive native open on macOS is
therefore not run by the smoke. Listing the folder and the scan policy use
only `std::fs` (case-insensitive APFS sorts names by bytes like the other
systems; links are skipped); the policy tests with symbolic links and the
executable bit run on macOS in CI. **Not run on macOS yet.**

## Searching inside a command (#30)

The smoke's search phase (screenshots 160 to 169, [command search](../command-search.md#checks)),
with a data folder of its own, builds and starts the fixture service on a
free port of 127.0.0.1 (`fixture_service --port 0`, the port read from its
log) and installs Package search, the Rust search sample. "aurora" typed in
root search must leave the service's log without a request; opened, the
command's "Service address" form is set to the service; its search field
sends "aurora" (results listed, Enter shows a package's details); "slow" then "ember"
must log the held search as abandoned; the service's 503, then the service
stopped, are errors; restarted, a search lists results again.
**Not run on macOS yet.**

## Scheduled tasks (#47)

The smoke's scheduled task phase (screenshots 320 to 327, [scheduled tasks](../background.md#checks)),
with a data folder of its own (`schedule-data`), installs the Background
sample (Rust); nothing must have run (no count in its content). In Manage
extensions, Enter on **Schedule: Ticks** turns it on: it runs at once, the
row shows "Last run: Ticked 1 times" and `schedules.json` records the
answer. Opening Ticks and choosing "Wait 10 seconds in each run", then
turning the schedule off and on again, starts a run that waits (noted
"started"); disabling the package meanwhile stops it, and after ten more
seconds nothing was noted "finished" and the count did not move. Enabled
again, the row says "Last run stopped: Background sample was disabled" and
no run starts; turned off, `schedules.json` holds no task.
**Not run on macOS yet.**

## Continuing services (#48)

The smoke's service phase (screenshots 330 to 335, [continuing services](../background.md#continuing-services)),
with a data folder of its own (`service-data`), installs the Service sample
(Rust); nothing must have run (no beats in its content). In Manage
extensions, Enter on **Service: Heartbeat** starts it: the row shows
"Running · Beat N", `services.json` records it and, three seconds later, the
count has grown. Disabling the package stops it: for three beats' time the
count does not move. Enabled again, it starts again at once and the count
grows; Enter on its row stops it, the count stops, and `services.json` holds
no service.
**Not run on macOS yet.**

## Text input and accessibility findings

- **Text input / IME (#20):** extension forms now have a text field. The
  smoke also opens the Rust command's form, submits it empty (the error color
  must appear), types "Ada" through System Events `keystroke`, then Tab, Down
  and Return (the result color must appear, which only happens if the typed
  text reached the name field). In run [36371205770](https://github.com/wasimysaid/pane/actions/runs/36371205770) (macOS 15.7.9, arm64) every step passed and the result read "Good morning, Ada, from the Rust guest". Composition with a macOS input method (for example
  Japanese Kana) through `NSTextInputClient` is unverified; the window tests
  cover composition only on the field's editing state
  ([what that proves](../forms.md#checks)). Editing bindings
  follow the element's macOS defaults (Cmd-A/C/V/X/Z, Option-arrow words); only typing, Tab and the arrow keys ran natively.

Screenshots from run [36371205770](https://github.com/wasimysaid/pane/actions/runs/36371205770) (commit `949e35d`), cropped to the window:

| Step | Evidence |
| --- | --- |
| Form opened; focus in the name field | [6-form.png](evidence/macos/6-form.png) |
| Submitted empty; "Enter a name" on the field and status | [7-form-error.png](evidence/macos/7-form-error.png) |
| Typed "Ada", Tab, Down to "Good morning", submitted | [8-form-result.png](evidence/macos/8-form-result.png) |

- **Accessibility:** the window exposes a `ListBox` labelled with the view
  title, `ListBoxOption` rows with label, description and selected state, the
  selected row as the active descendant, and a `Status` node for the result.
  This is verified through GPUI's accessibility tree in the window tests
  (`assistive_technology_sees_the_list_the_selection_and_the_result`), which
  pass on macOS but are platform-independent. **VoiceOver was not run**, so how
  the tree reaches NSAccessibility and what VoiceOver announces are unverified.
  Forms: see [accessibility of forms](../forms.md#accessibility).
- **Custom view (#21):** after a final restart the smoke opens the Rust command's
  color picker, presses Right (key code 124) and clicks the dark green swatch
  with a Quartz mouse event posted through Python `ctypes`, converting the
  screenshot's pixels to points (half on Retina). Each step must show the
  chosen color over at least 3000 pixels. In run [36378453278](https://github.com/wasimysaid/pane/actions/runs/36378453278) (commit `1487dc8`, macOS 15.7.9, arm64) every step passed: the picker opened on blue (#1E88E5), Right moved to purple (#8E24AA) and the click chose dark green (#1B5E20); posting the Quartz event needed no permission beyond the one System Events has. Accessibility: see
  [custom views](../custom-views.md#accessibility).
- **Operations (#22):** the operations phase (screenshots 31 and 32) installs
  the JavaScript operations sample, then the Rust one, opens the Rust
  sample's command and fills its form with the JavaScript package's identity
  and the name "Rust"; the Rust guest calls that package's `greet` operation
  through Pane, the same steps as on
  [Linux](linux.md#operations-22). In run
  [36429153309](https://github.com/wasimysaid/pane/actions/runs/36429153309)
  (commit `1848494`) every step passed: the Rust guest's call into the
  JavaScript package answered "Hello, Rust, from JavaScript"
  ([32-operation-answer.png](evidence/macos/32-operation-answer.png)).
- **Reload (#11):** after the calculator and operations phases, the smoke installs a package from
  `<output-dir>/dev`, replaces its component with the JavaScript sample and
  reloads it in Manage extensions, then reloads it without its component (the
  checks fail and the old code keeps answering) and with the `failing-start`
  fixture (a startup failure, then Retry); screenshots 33 to 39, the same
  steps as on [Linux](linux.md#reloading-a-package-11). In run
  [36429153309](https://github.com/wasimysaid/pane/actions/runs/36429153309)
  (commit `1848494`) every step passed: reloaded with the JavaScript build,
  Dev answered from the new code
  ([35-dev-after.png](evidence/macos/35-dev-after.png)); reloading the
  `failing-start` fixture showed "Reloaded Dev, but it failed to start…
  Retry…" ([38-start-failed.png](evidence/macos/38-start-failed.png)), and
  Retry then showed "Started Dev"
  ([39-retried.png](evidence/macos/39-retried.png)).
- **Clearing an extension's cache (#39):** after the reload phase, the smoke
  restarts Pane, saves one value of each kind of
  [extension data](../extension-data.md) for the Settings sample (style,
  content, cache and credential), then chooses "Clear cache of Settings
  sample" in Manage extensions and confirms, the same steps as on
  [Linux](linux.md#clearing-an-extensions-cache-39). In run
  [36429153309](https://github.com/wasimysaid/pane/actions/runs/36429153309)
  (commit `1848494`) every step passed: Manage extensions showed "Cleared the
  cache of Settings sample"
  ([42-cache-cleared.png](evidence/macos/42-cache-cleared.png)), and "Show
  what Pane keeps" then read "Style: formal · Note: Water the plants ·
  Signed in: yes · Cached greeting: none", with the cached greeting gone and
  the other three values kept
  ([43-kept-after-clear.png](evidence/macos/43-kept-after-clear.png)).

## Disabling an extension and keeping its settings (#10)

In run [36378453278](https://github.com/wasimysaid/pane/actions/runs/36378453278) (commit `1487dc8`) the disable phase passed: the Settings sample
saved the formal greeting, was disabled in Manage extensions, stayed disabled
and absent from root search after a restart (the root screenshot matches the
one taken before the package was installed), was enabled again, and "Greet me"
answered "Good day to you" from the kept setting.

| Step | Evidence |
| --- | --- |
| Formal greeting saved | [16-setting-saved.png](evidence/macos/16-setting-saved.png) |
| Disabled in Manage extensions | [17-disabled.png](evidence/macos/17-disabled.png) |
| After a restart: Greeting absent from root | [18-restarted-disabled.png](evidence/macos/18-restarted-disabled.png) |
| Enabled again | [19-enabled.png](evidence/macos/19-enabled.png) |
| The kept setting answers | [20-greeted.png](evidence/macos/20-greeted.png) |

Custom view screenshots from the same run: [21-color.png](evidence/macos/21-color.png),
[22-color-key.png](evidence/macos/22-color-key.png),
[23-color-click.png](evidence/macos/23-color-click.png).

## Local extension package (#9)

`scripts/smoke-macos.sh` also installs `target/guests/packages/sample-rust` with
`pane --install <folder>` (with `PANE_DATA_DIR` pointing at a fresh folder),
runs its command, and restarts Pane. In run [36371205770](https://github.com/wasimysaid/pane/actions/runs/36371205770) (commit `949e35d`) every
step passed. The `packages` identity tests also passed there: folder paths with
spaces and Unicode, letter case and Unicode normalization as this file system
treats them, and symbolic links.

| Step | Evidence |
| --- | --- |
| Package screen: source, version, commands, compatibility | [9-package.png](evidence/macos/9-package.png) |
| Installed; the new command is selected in root search | [10-installed.png](evidence/macos/10-installed.png) |
| The installed command answers ("Hello from the Rust guest") | [11-installed-result.png](evidence/macos/11-installed-result.png) |
| Still listed after a restart | [12-restarted.png](evidence/macos/12-restarted.png) |

The installed copy has the same title as the built-in Rust sample, so the
screenshots can't show which copy opened; the core tests prove the installed
copy runs. In these screenshots the root list is taller than the window and its
last row is cut off; since #19 the list scrolls to keep the selected row
visible.

## Remaining limits

- Only a CI runner was used; no one has run it on a contributor's own Mac.
- Intel Macs, other macOS versions and the JS/TS toolchain build on macOS are
  untested. The wasi-sdk checksum for macOS in `tools/componentize-js/pins.json`
  has not been checked against a real download.
- The smoke confirms that text appears in the expected colors and that the
  three results differ. Which command and guest each screenshot shows was
  checked by inspection.
