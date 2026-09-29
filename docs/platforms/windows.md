# Windows native baseline (#5, #6)

Recorded 2026-09-28 from GitHub Actions run
[36366760796](https://github.com/wasimysaid/pane/actions/runs/36366760796) on the
fork `wasimysaid/pane`, commit `572d629`.

## Tested combination

| | |
| --- | --- |
| OS | Windows Server 2025 Datacenter, `Microsoft Windows NT 10.0.26100.0` (build 26100) |
| Architecture | x86_64 (`AMD64`) |
| Machine | GitHub-hosted runner, image `windows-2025-vs2026` version 20260922.246.2 |
| Session | the runner's interactive desktop session (Explorer shell, taskbar) |
| Toolchain | MSVC from the image's Visual Studio; Rust 1.98.1 (`rust-toolchain.toml`) |

**Not tested, so not claimed:** Windows 10/11 client editions, ARM64, a physical
display or GPU driver other than the runner's, high-DPI scaling, and any
installer or signed build.

## Fresh checkout build and checks

```powershell
rustup toolchain install
cargo xtask ci          # guests, prebuilt JS/TS check, fmt, clippy, all tests
cargo build --locked -p pane
```

Result: `cargo xtask ci` passed (13 window, 9 launcher-model, 1 runtime-cache
and 25 sample-contract tests), and the launcher built. The Rust guests are built
natively; the JS and TS samples use the committed prebuilt components.
Rebuilding them from source on Windows (`cargo xtask js-guests`) has **not**
been run. The Cargo cache was warm (`Swatinem/rust-cache`).

## Native GUI smoke

```powershell
cargo build -p pane
./scripts/smoke-windows.ps1 -OutDir smoke   # needs Python 3 with Pillow
```

The script launches `target\debug\pane.exe` and waits for its main window. It
then brings the window to the foreground and sends real key events with
`SendKeys`. For each of the Rust, JavaScript and TypeScript sample commands it
presses Enter to open it, Down and Enter to run "Wait briefly" (an async WASI 0.3
clock import inside the guest), then Escape.

It fails if the window does not appear or Pane exits. The same three screenshot
checks as on macOS and Linux also run (`scripts/check_screenshot.py`):

- The root screen draws text in the hint color.
- Each result screen draws text in the result color.
- The three result screens are all different.

Screenshots, cropped to the window (and inspected):

| Step | Evidence |
| --- | --- |
| Root search lists the three sample commands | [1-root.png](evidence/windows/1-root.png) |
| Rust command opened; action result "Waited 50 ms inside the Rust guest" | [2-command-0.png](evidence/windows/2-command-0.png), [2-result-0.png](evidence/windows/2-result-0.png) |
| JavaScript command; "Waited 50 ms inside the JavaScript guest" | [3-command-1.png](evidence/windows/3-command-1.png), [3-result-1.png](evidence/windows/3-result-1.png) |
| TypeScript command; "Waited 50 ms inside the TypeScript guest" | [4-command-2.png](evidence/windows/4-command-2.png), [4-result-2.png](evidence/windows/4-result-2.png) |
| Escape returns to root search | [5-back-to-root.png](evidence/windows/5-back-to-root.png) |

Rendering, keyboard focus, selection, guest execution and result display all
worked natively. In the root screenshots the TypeScript row also has a lighter
background. This is most likely hover under wherever the runner's mouse pointer
sits, since keyboard selection (the Rust row) opened the Rust command. This was
not confirmed.

## Platform availability (#19)

The smoke also runs the platform-availability steps (screenshots 13 to 15,
[platform availability](../platform-availability.md#checks)): the Rust
command's Windows-only and macOS-and-Linux actions, then a package listing
only the two other systems. In run [36372625940](https://github.com/wasimysaid/pane/actions/runs/36372625940) (commit `38a95cb`, Windows NT 10.0.26100, AMD64) every step passed: the Windows-only action answered, the macOS-and-Linux action was listed with "Not available on Windows: this action supports only macOS and Linux" and did not run, and the package for macOS and Linux was refused with "Not available on Windows: this package supports only macOS and Linux". The list scrolled to keep the selected row visible. The later #19 fixes (per-command platforms, re-focusing before these steps) have not run here yet.

| Step | Evidence |
| --- | --- |
| Windows-only action | [13-windows-only.png](evidence/windows/13-windows-only.png) |
| macOS-and-Linux action | [14-not-windows.png](evidence/windows/14-not-windows.png) |
| Package for the other two systems | [15-no-compatible-package.png](evidence/windows/15-no-compatible-package.png) |

## Root search (#23)

Root search has a query field with focus ([root search](../root-search.md)).
The smoke's search phase (screenshots 24 to 26) types "typescr" with
`SendKeys`, opens the only match and runs "Wait briefly", which must look
exactly like step 4, then types "zzz" and presses Enter on no results. In run [36420611977](https://github.com/wasimysaid/pane/actions/runs/36420611977) (commit `6d73d18`) every step passed: typing "typescr" left only TypeScript sample and Enter ran it, and "zzz" showed No results ([24-search.png](evidence/windows/24-search.png), [26-no-results.png](evidence/windows/26-no-results.png)). Input-method composition in the query field is still unverified here.

## Calculator (#27)

The smoke's calculator phase (screenshots 27 to 30) installs the calculator
package, types "6*7", checks the selected answer row, presses Enter to copy
it, then compares typing "42+1" with pasting the copy (Ctrl+A, Ctrl+V through `SendKeys`) and typing
"+1", which must look the same. In run [36423871204](https://github.com/wasimysaid/pane/actions/runs/36423871204) (commit `ab91081`) every step passed: "6*7" answered 42, Enter copied it, and pasting then typing "+1" matched typing "42+1", so the system clipboard held "42" ([27-answer.png](evidence/windows/27-answer.png), [30-pasted.png](evidence/windows/30-pasted.png)).

## Applications (#24)

[Applications](../applications.md) finds the `.lnk` shortcuts in the user's
and all users' Start menu Programs folders and opens one with
`ShellExecuteEx`, as Explorer does, plus the packaged (AppX/MSIX) apps of
the shell's Apps folder, such as Calculator on Windows 11, opened by their
AppUserModelID; a native test requires an inbox packaged app (Calculator or
Settings) to be found. The smoke's
last phase (screenshots 44 and 45) makes a shortcut "Pane Smoke App" to
`cmd.exe` writing a marker file (with `WScript.Shell`, minimized) under an
APPDATA given to Pane only, installs the package, types "pane smoke",
checks the selected row, presses Enter and checks "Opened Pane Smoke App"
and the marker; the adapter tests open such a shortcut too. **Not run on
Windows yet**: this branch was not pushed, so the phase, the native tests
and the `ShellExecuteEx` path are unverified here (the Windows code was
only type-checked and linted for `x86_64-pc-windows-gnu`).

## Quicklinks (#28)

The smoke's last phase (screenshots 46 and 47) installs the Quicklinks
package, creates "Pane issues" (https://example.com/pane-issues) in its
form, restarts Pane and types "pane iss", which must list it selected. It
stops before Enter, which would open the default browser; opening a link
here is checked only through the tests' recording opener. Not run yet.

## Global hotkeys (#32)

The smoke's hotkey phase (screenshots 52 to 58, [global hotkeys](../hotkeys.md#checks))
assigns Ctrl+Alt+G to Greeting on its hotkey screen, minimizes Pane,
presses it with `SendKeys` and checks that Pane's window is the foreground
window again with Greeting open; then again after a restart; then, with the
extension disabled, that pressing it leaves Pane minimized and unchanged.
`RegisterHotKey` needs no permission. `hotkey_adapters.rs` registers a
shortcut on the test session, checks that a second registration is refused
as taken and that the released shortcut can be registered again. The
adapter was only compile- and lint-checked for `x86_64-pc-windows-gnu` from
Linux; **not run on Windows yet**, so registration, delivery and the focus
transition (foreground rules) are unverified natively.

## Deleting retained data (#41)

The smoke's retained-data phase (screenshots 63 to 65, [deleting retained
data](../extension-data.md#deleting-retained-data)), with a data folder of
its own, saves a note with the settings sample, uninstalls it keeping its
saved data, deletes its retained data from the extension list's last row
(Down from the selected Cancel, then Enter), checks that `installed.json`
and `content.json` no longer hold it, and reinstalls the same folder, which
must show nothing kept. **Not run on Windows yet.** A data file locked by
another program is covered only by the tests' unreadable files, and a
record that cannot be written after the data is deleted only by a Unix test.

## Aliases and fallbacks (#31)

The smoke's alias phase (screenshots 66 to 74, [aliases and fallbacks](../aliases.md#checks))
gives Echo, the query sample's command, the alias "ec" and makes it a
fallback in Manage extensions, sends "ec hello" and, from the unselected
fallback row chosen with Down, "zqx" to it, and checks that with the
extension disabled "ec hello" lists nothing. Nothing in it is specific to
Windows (no system API is involved); **not run on Windows yet**.

## Dependencies (#42)

The dependencies phase (screenshots 75 to 77, [dependencies](../dependencies.md#checks)),
with a data folder of its own, previews the dependencies sample (its
required JavaScript operations sample and optional Rust one listed),
installs it with the JavaScript sample and runs "Greet through the required
greeter", which must answer from the JavaScript guest; `installed.json`
must then hold exactly two packages and the recorded dependency. A
dependency's `local:../…` source is joined to the package folder and
resolved with the same `canonicalize` (without the `\\?\` prefix) as
package identity; a folder that does not exist is resolved from its
spelling. **Not run on Windows yet**, so `..` across drive-letter and UNC
paths is untested natively.

## Native helpers (#15)

The first native-helper phase (screenshots 90 to 93, data folder `helper-data`,
[native helpers](../helpers.md#checks)) installs the helper sample, whose
`pane-echo.exe` `cargo xtask guests` builds for the runner
(`windows-x86_64` on `windows-2025`), runs it (the answer must name Windows
x86-64), cancels a slow run after a second, starts the ten-second run,
checks with `Get-Process` that the helper runs from the managed copy,
disables the package and checks that the process is gone, that the saved
"started" note is kept, and that no helper outlives Pane. A second phase
(screenshot 94, `helper-quit-data`) starts the waiting helper, closes
Pane's window with `CloseMainWindow` (WM_CLOSE), and checks that Pane
exits, no helper runs and its heartbeat file stops growing. The tests in
`crates/pane-core/tests/helpers.rs` (Rust, JavaScript and TypeScript
samples; a PE header) and the runner's unit tests (the `.exe` rule and
absolute path are checked on every system) run in `cargo xtask ci` there,
against the `pane-echo.exe` built natively on the runner. Pane
starts a helper without a console window (`CREATE_NO_WINDOW`) and ends it
with `TerminateProcess`; a helper's own children are not in a job object.
The runner was only compile- and lint-checked for `x86_64-pc-windows-gnu`
from Linux; **not run on Windows yet**, so starting, ending and reaping a
helper natively, and the smoke's process checks, are unverified there. An
update ends the old copy's helpers before removing its folder, so the
folder is not in use; the removal at the next start remains a fallback.
No Windows arm64 build was made.

## Development mode (#12, #13)

The smoke's last phase (screenshots 110 to 136, [development
mode](../development-mode.md#checks)) builds a copy of each development
sample, develops it from Manage extensions, saves an edit, a change that
does not build, two saves in a row and, after stopping, one more, checking
the answers, the error and that nothing is built after stopping. The
TypeScript and JavaScript samples run only where the JS toolchain is built,
so CI's smoke runs the Rust one. The platform code (`ReadDirectoryChangesW` through notify, a Job Object
with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, `CREATE_NEW_PROCESS_GROUP` and
builds without a console window) was only compile- and lint-checked for
`x86_64-pc-windows-gnu` from Linux; **not run on Windows yet**, so the file
watcher's events, the build's processes being killed and the whole phase
are unverified natively.

## Disabling required dependents (#43)

The disable-dependents phase (screenshots 140 to 143, [disabling a required dependency](../dependencies.md#disabling-a-required-dependency)),
with a data folder of its own, installs the dependencies sample, presses
Enter on the JavaScript operations sample in Manage extensions, which must
ask first (the details color), cancels, then chooses Disable all (the
result color) and enables the JavaScript sample again alone (the result
color); `installed.json` must then record exactly one disabled package.
Nothing in it is specific to Windows (no system API is involved; the
closure reuses the dependency identities recorded at install); **not run on
Windows yet**.

## Runtime crashes (#17)

The runtime-crash phase (screenshots 200 to 209, data folder
`runtime-crash-data`, [runtime crashes](../pausing.md#when-the-extension-runtime-itself-crashes))
installs the helper and settings samples, starts Pane with
`PANE_TEST_RUNTIME_FAULTS` naming a fault file, runs the settings sample's
Count, starts the waiting helper and has the runtime crash: the helper must
be gone (`Get-Process`, and its heartbeat must stop growing), the note it saved kept,
and the status line the error color. A second crash, injected before
Count's answer, must leave the count at 2 and the runtime stopped; root
search explains it, Manage extensions shows why (the details color), a
disable works, **Restart the extension runtime** runs extensions again and
Count then counts 3; no package may be recorded as paused. Nothing in it is
specific to Windows (the runtime is a thread; helpers are ended as for a
disable); **not run on Windows yet**.

## Uninstalling required dependents (#44)

A phase of the smoke (screenshots 180 to 183, [uninstalling a required dependency](../dependencies.md#uninstalling-a-required-dependency)),
with a data folder of its own, installs the dependencies sample, presses
Enter on "Uninstall JavaScript operations sample" in Manage extensions,
which must ask first (the details color), cancels, then chooses Uninstall
all 2 keeping saved data (the result color); `installed.json` must then hold
no package. Pane is started again to install the JavaScript operations
sample alone, and `installed.json` must then hold exactly one package.
Removing a managed copy uses the same `remove_dir_all` as a single
uninstall; a folder still in use (a file open on Windows) is listed and removed at the next start,
reported against its own package. Nothing else in it is specific to
Windows; **not run on Windows yet**.

## npm packages (#45)

A phase of the smoke (screenshots 260 to 266, [npm packages](../npm.md)),
with a data folder of its own, starts `scripts/npm_registry.py` (with
`python`) on 127.0.0.1 serving the npm sample `cargo xtask guests` packed,
and points the development build at it with `PANE_NPM_REGISTRY`: it installs
the local Dependencies from npm sample, which downloads and installs the npm
package it requires, calls its `greet` operation, then names the npm package
in "Install extension from npm…" (Up from the last row; SendKeys types
`@pane-samples/greeter`), updates it and runs its command, which answers
"Hello from the npm package"; `installed.json` must then record `"npm":
"@pane-samples/greeter"` at `"npmVersion": "0.1.0"` with both packages.
Unpacking refuses, on every system alike, the names Windows reads
differently or cannot write: `\ : < > " | ? *`, control characters,
trailing dots and spaces, and device names such as `con`, `conin$`,
`conout$`, `com1` or `lpt³` (compared by character, with any extension), so
a tarball Linux accepts never fails or writes elsewhere on Windows; the
real registry would be reached through the HTTP client guests' requests
use, trusting the certificates rustls-native-certs reads from the Windows
certificate store, which no check exercises (the smoke never reaches the
network; [by hand](../npm.md#trying-the-real-registry-by-hand), not run). The packing, in `cargo xtask guests`, runs in CI on
Windows. **Not run on Windows yet.**

## Files (#29)

The files phase (screenshots 220 to 223, [files](../files.md)), with a data
folder of its own, installs Files and presses Enter on Pane's own "Choose
folder…" row; a debug build's `PANE_TEST_CHOOSE_FOLDER` names a fixture
folder "Pane smoke files" (spaces) in the smoke's output folder instead of
showing the system's picker. It types "plan", which must list "Résumé plan
ü.txt" selected, and presses Enter. The real opener (PowerShell's
`Invoke-Item -LiteralPath` for an existing path, through the `open` crate,
the path passed in an environment variable) can show the "Open with" dialog
for a type with no handler, so a debug build's `PANE_TEST_OPEN_FILE_LOG`
makes it record the path in a file instead; the recorded path, resolved,
must be the fixture file's, resolved. Last it types "runner" and presses
Enter on a batch file, which Pane must refuse without recording or running
it. A positive native open on Windows is therefore not run by the smoke. The
scan policy skips entries with the hidden or system attribute and junctions
(reparse points) on Windows only, and a grant refuses UNC paths from their
text before any file system call; a Windows-only test (`attrib +h`,
`mklink /J`) is written but has not run. **Not run on Windows yet.**

## Searching inside a command (#30)

The smoke's search phase (screenshots 160 to 169, [command search](../command-search.md#checks)),
with a data folder of its own, builds and starts the fixture service on a
free port of 127.0.0.1 (`fixture_service --port 0`, the port read from its
log) and installs Package search, the Rust search sample. "aurora" typed in
root search must leave the service's log without a request; opened, the
command's "Service address" form is set to the service; its search field
sends "aurora" (results listed, Enter shows a package's details); "slow" then "ember"
must log the held search as abandoned; the service's 503, then the service
stopped, are errors; restarted, a search lists results again. Windows retries a refused connection for about two seconds, so the offline step waits longer.
**Not run on Windows yet.**

## Clipboard history (#35)

The smoke's clipboard phase (screenshots 280 to 285, [clipboard history](../clipboard-history.md#checks)),
with a data folder of its own, installs Clipboard History and checks
`clipboard-history.json` at each step: text copied before it is turned on is
not kept; once turned on (its first row) plain text is kept, while text
carrying `ExcludeClipboardContentFromMonitorProcessing`,
`CanIncludeInClipboardHistory` = 0 or `CanUploadToCloudClipboard` = 0 is
not; nothing is kept while paused, or while disabled, also after a restart;
Enter on a kept item puts it on the clipboard again and moves it to the
front; enabled again, text is kept, also after a restart, before the command
is opened. The smoke copies only its own `pane-smoke-...` text, through the
clipboard API from PowerShell, and so replaces what was on the clipboard,
without reading or restoring it. `clipboard_adapter.rs` checks the adapter
alone: plain text reported with its owner (the test's process), each marker
read and withholding the text, a written text reported, and nothing once
the watch is dropped; it too replaces the clipboard, so it runs only with
`PANE_TEST_REAL_CLIPBOARD=1`, which CI's Windows job sets. `atomic.rs`'s
Windows unit test checks the owner-only DACL of `clipboard-history.json`
and `credentials.json`.

The adapter, the shared message thread (also the hotkey adapter's), the
DACL and these tests were only compile- and lint-checked for
`x86_64-pc-windows-gnu` from Linux; **not run on Windows yet**. They run in
CI (`cargo xtask ci` with `PANE_TEST_REAL_CLIPBOARD=1`, then
`smoke-windows.ps1`) on `windows-2025`, and the next green Windows run of
the branch is their evidence: until then the listener's delivery, the
retry and stop paths, the markers as real password managers set them, the
owner lookup and the DACL are unverified natively.

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
**Not run on Windows yet.**

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
**Not run on Windows yet.**

## Text input and accessibility findings

- **Text input / IME (#20):** the smoke now also opens the Rust command's
  form, submits it empty (the error color must appear), types "Ada" with
  `SendKeys`, then Tab, Down and Enter (the result color must appear, which
  only happens if the typed text reached the name field). In run [36371205770](https://github.com/wasimysaid/pane/actions/runs/36371205770) (Windows NT
  10.0.26100, AMD64) every step passed and the result read "Good morning, Ada,
  from the Rust guest". Windows IME
  (TSF) composition, for example with Microsoft Japanese IME, is unverified;
  the window tests cover composition only on the field's editing state
  ([what that proves](../forms.md#checks)).

Screenshots from run [36371205770](https://github.com/wasimysaid/pane/actions/runs/36371205770) (commit `949e35d`), cropped to the window:

| Step | Evidence |
| --- | --- |
| Form opened; focus in the name field | [6-form.png](evidence/windows/6-form.png) |
| Submitted empty; "Enter a name" on the field and status | [7-form-error.png](evidence/windows/7-form-error.png) |
| Typed "Ada", Tab, Down to "Good morning", submitted | [8-form-result.png](evidence/windows/8-form-result.png) |

- **Accessibility:** see [accessibility of forms](../forms.md#accessibility)
  and [of custom views](../custom-views.md#accessibility). Narrator/NVDA were
  not run.
- **Custom view (#21):** after a final restart the smoke opens the Rust command's
  color picker, presses Right and clicks the dark green swatch with `user32`
  `SetCursorPos` and `mouse_event`, at the screenshot's pixel position. Each
  step must show the chosen color over at least 3000 pixels. In run [36378453278](https://github.com/wasimysaid/pane/actions/runs/36378453278) (commit `1487dc8`, Windows NT 10.0.26100, AMD64) every step passed: the picker opened on blue (#1E88E5), Right moved to purple (#8E24AA) and the click chose dark green (#1B5E20). The runner displays at 100 % scaling, so other scaling is still unverified. The script calls `SetProcessDPIAware` first, so
  the screenshot, the screen bounds and `SetCursorPos` all use physical
  pixels and the click should land on the swatch at any display scaling;
  scaling other than 100 % is unverified.
- **Operations (#22):** the operations phase (screenshots 31 and 32) installs
  the JavaScript operations sample, then the Rust one, opens the Rust
  sample's command and fills its form with the JavaScript package's identity
  and the name "Rust"; the Rust guest calls that package's `greet` operation
  through Pane, the same steps as on
  [Linux](linux.md#operations-22). In run
  [36429153309](https://github.com/wasimysaid/pane/actions/runs/36429153309)
  (commit `1848494`) every step passed: the Rust guest's call into the
  JavaScript package answered "Hello, Rust, from JavaScript"
  ([32-operation-answer.png](evidence/windows/32-operation-answer.png)).
- **Reload (#11):** after the calculator and operations phases, the smoke installs a package from
  `<output-dir>\dev`, replaces its component with the JavaScript sample and
  reloads it in Manage extensions, then reloads it without its component (the
  checks fail and the old code keeps answering) and with the `failing-start`
  fixture (a startup failure, then Retry); screenshots 33 to 39, the same
  steps as on [Linux](linux.md#reloading-a-package-11). In run
  [36429153309](https://github.com/wasimysaid/pane/actions/runs/36429153309)
  (commit `1848494`) every step passed: reloaded with the JavaScript build,
  Dev answered from the new code
  ([35-dev-after.png](evidence/windows/35-dev-after.png)); reloading the
  `failing-start` fixture showed "Reloaded Dev, but it failed to start…
  Retry…" ([38-start-failed.png](evidence/windows/38-start-failed.png)), and
  Retry then showed "Started Dev"
  ([39-retried.png](evidence/windows/39-retried.png)). Replacing the managed
  copy removes the old folder on a best-effort basis; on Windows a folder
  still in use is left behind, listed in `installed.json`, and removal is
  tried again at the next start (tested on Linux with a folder whose files
  cannot be deleted; not run on Windows).
- **Clearing an extension's cache (#39):** after the reload phase, the smoke
  restarts Pane, saves one value of each kind of
  [extension data](../extension-data.md) for the Settings sample (style,
  content, cache and credential), then chooses "Clear cache of Settings
  sample" in Manage extensions and confirms, the same steps as on
  [Linux](linux.md#clearing-an-extensions-cache-39). In run
  [36429153309](https://github.com/wasimysaid/pane/actions/runs/36429153309)
  (commit `1848494`) every step passed: Manage extensions showed "Cleared the
  cache of Settings sample"
  ([42-cache-cleared.png](evidence/windows/42-cache-cleared.png)), and "Show
  what Pane keeps" then read "Style: formal · Note: Water the plants ·
  Signed in: yes · Cached greeting: none", with the cached greeting gone and
  the other three values kept
  ([43-kept-after-clear.png](evidence/windows/43-kept-after-clear.png)).

## Disabling an extension and keeping its settings (#10)

In run [36378453278](https://github.com/wasimysaid/pane/actions/runs/36378453278) (commit `1487dc8`) the disable phase passed: the Settings sample
saved the formal greeting, was disabled in Manage extensions, stayed disabled
and absent from root search after a restart (the root screenshot matches the
one taken before the package was installed), was enabled again, and "Greet me"
answered "Good day to you" from the kept setting.

| Step | Evidence |
| --- | --- |
| Formal greeting saved | [16-setting-saved.png](evidence/windows/16-setting-saved.png) |
| Disabled in Manage extensions | [17-disabled.png](evidence/windows/17-disabled.png) |
| After a restart: Greeting absent from root | [18-restarted-disabled.png](evidence/windows/18-restarted-disabled.png) |
| Enabled again | [19-enabled.png](evidence/windows/19-enabled.png) |
| The kept setting answers | [20-greeted.png](evidence/windows/20-greeted.png) |

Custom view screenshots from the same run: [21-color.png](evidence/windows/21-color.png),
[22-color-key.png](evidence/windows/22-color-key.png),
[23-color-click.png](evidence/windows/23-color-click.png).

## Local extension package (#9)

`scripts/smoke-windows.ps1` also installs `target/guests/packages/sample-rust` with
`pane --install <folder>` (with `PANE_DATA_DIR` pointing at a fresh folder),
runs its command, and restarts Pane. In run [36371205770](https://github.com/wasimysaid/pane/actions/runs/36371205770) (commit `949e35d`) every
step passed. The `packages` identity tests also passed there: folder paths with
spaces and Unicode, letter case and Unicode normalization as this file system
treats them, and symbolic links. The symbolic-link test skips itself where
directory links are not allowed, and cargo hides that notice for a passing
test. The runner's administrator account can normally create them, but a skip
can't be ruled out from the log.

| Step | Evidence |
| --- | --- |
| Package screen: source, version, commands, compatibility | [9-package.png](evidence/windows/9-package.png) |
| Installed; the new command is selected in root search | [10-installed.png](evidence/windows/10-installed.png) |
| The installed command answers ("Hello from the Rust guest") | [11-installed-result.png](evidence/windows/11-installed-result.png) |
| Still listed after a restart | [12-restarted.png](evidence/windows/12-restarted.png) |

The installed copy has the same title as the built-in Rust sample, so the
screenshots can't show which copy opened; the core tests prove the installed
copy runs. In these screenshots the root list is taller than the window and its
last row is cut off; since #19 the list scrolls to keep the selected row
visible.

## Remaining limits

- Only a CI runner (Windows Server) was used, not a Windows 10/11 desktop.
- No screen reader (Narrator/NVDA) was run. The accessibility tree is verified
  only through GPUI in the platform-independent window tests.
- The smoke confirms that text appears in the expected colors and that the
  three results differ. Which command and guest each screenshot shows was
  checked by inspection.
