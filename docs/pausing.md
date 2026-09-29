# Pausing a broken extension

Added for [#16](https://github.com/hoangvu12/pane/issues/16) (US77, US78,
US79, US80, T17, G3; contributions, not a claim that the whole scenario or
gate passes). When one installed extension keeps failing, Pane pauses it,
says so, and offers Retry, while the rest of Pane keeps running. No command
line is needed. The model builds on [generations](generations.md): pausing
ends the package's generation.

## What pauses a package

Only a failure Pane can attribute to one package: its code, in the
package's current generation, failed on its own.

| Failure | Paused | Why |
| --- | --- | --- |
| A component could not be loaded or instantiated, or a reload's new code trapped as it started (a [startup failure](../CONTEXT.md)) | At once | Starting it again would fail the same way. |
| A guest call trapped (a crash): opening a command, an action, a form, a custom view's event, drawing or destructor, root results, indexed results, a query sent to a command through its alias or as a fallback, an operation another package called, a [scheduled task](background.md)'s run in the background (#47), or a [continuing service](background.md#continuing-services) (#48), which Pane starts again a minute after a crash unless it paused the package | On the **3rd crash within 5 minutes** | A broken command stops failing soon; one bad input does not stop an extension that otherwise works. |

These are explicit choices, not measurements (`CRASHES_BEFORE_PAUSE` and
`CRASH_WINDOW` in
[`launcher/pausing.rs`](../crates/pane-core/src/launcher/pausing.rs)):

- Calls that answer between crashes **do not** start the count again: a
  user opens the command (which answers) before each crashing action, so a
  reset on any answer would never pause it. Crashes more than 5 minutes
  apart are not counted together, so rare crashes of a long-running Pane
  never add up to a pause.
- The count is kept in memory for the package's current generation: a
  restart, disable, enable, reload, update or Retry starts it afresh. Only
  the pause itself is recorded. The window is tested with explicit times
  (`Pauses::crashed` takes the time of each crash).

What is **not** a failure of the package:

- An **error the extension answers with** ("sign in first", a refused form,
  a refused view): an expected outcome of code that runs. It never pauses
  it, however often it happens.
- A **call stopped** because its generation, or that of a caller in its
  chain, ended (disable, reload, update, uninstall), and a scheduled task's
  run stopped the same way or because the user turned its schedule off, and
  a continuing service stopped the same way or by the user. A service's
  error or loss to a runtime crash restarts it a minute later, never
  counting towards a pause. A package serving an
  operation for a stopped caller loses its instance and restarts afresh on
  its next call, as after a crash ([generations](generations.md#what-stopping-costs));
  that is not counted.
- A **crash of an operation's target** counts for the target only; its
  caller answered.
- A crash reported after its package was disabled, reloaded, updated or
  uninstalled: the package's generation is checked with the launcher's
  state locked, the same lock those end it under, so such a report never
  counts.
- In JavaScript and TypeScript, **anything a handler throws** is an error
  it answers with: the build wraps the exported handlers
  ([`guests/js/adapt.js`](../guests/js/adapt.js)), so a thrown `Error`
  from `submitForm` rejects the form as a whole rather than trapping.
- A failure of the **runtime itself**, with no attributable package, pauses
  nothing: a reload or Retry that cannot start the package because Pane's
  runtime is unavailable (it could not start, or its thread has stopped)
  says so, and a Retry leaves the pause as it was. Recovering from a crash
  of the runtime thread is described [below](#when-the-extension-runtime-itself-crashes)
  (#17); a guest that stops responding is
  [#18](https://github.com/hoangvu12/pane/issues/18).

## What a paused package does

- Its generation ends (`End::Paused`): its pending calls stop, its
  instances and views are dropped, and its code can no longer read or save
  data or call operations. A command of it that is open closes.
- Its commands stay in root search, listed and selectable, saying "<title>
  is paused after an error; retry it in Manage extensions"; activating one
  shows that and runs nothing. A global hotkey assigned to one stays
  registered (it is the user's choice); pressing it shows the same reason.
  It computes no root results and supplies no indexed ones. Its commands'
  alias and fallback rows ([aliases](aliases.md)) stay listed with the same
  reason and send nothing. Another
  package calling its operations is answered `unavailable` with the same
  reason. Root search tells a paused command from one this system does not
  support by its reason's kind (`Unavailable::Paused` against
  `Unavailable::OnThisSystem`).
- The status line (the launcher's toast) says "<title> crashed 3 times
  within 5 minutes and is paused" (or "could not start and is paused"),
  that its saved data is kept, and that Retry and why are in Manage
  extensions.
- **Manage extensions…** lists it as "Enabled · Paused after crashing" (or
  "Enabled · Failed to start"), with a **Retry <title>** row (**Retry
  starting <title>** after a failure to start) and a **Why <title> is
  paused** row. That opens the details: how it failed, its source and
  version, what is kept, and the full diagnostics (the last crash's message
  and backtrace), with Retry. The diagnostics of a reload that fails to
  start also go to standard error.
- Its settings, content, cache and credentials are kept. Clearing its cache,
  uninstalling it and the rest of Manage extensions work, since none of them
  runs it.
- Other packages keep running.

Paused is **not** disabled: disabled is the user's choice and adds nothing
to root search; paused is Pane's, and says why.

## Persistence and how a pause ends

The pause is recorded in `installed.json` beside the package's record, with
the cause, the details, the package's version and its managed copy
(`"paused": { "after": "crashes", "why": …, "version": …, "code": "3" }`).
After a restart the package is still paused if its managed copy and version
are still those that failed: its commands are explained and nothing of it
runs, so a known broken package is not started again blindly.

The record is written by a thread of its own, in the order the pauses and
their ends happen, so neither the runtime thread (where a crash is noticed)
nor the window waits for the file; `Launcher::records_written` resolves
once what happened so far is written. A Pane stopped in the moment between a
pause and its record forgets that pause.

| Action | Effect |
| --- | --- |
| **Retry** | Starts the same code in a new generation, asking each available command for its view (as a reload does; see [current decisions](current-decisions.md) on this exception to lazy activation). If that fails to start, it is paused again; if Pane's runtime is unavailable, it stays paused as it was. The crash count starts afresh. |
| Reload or update | New code, which has not failed: the pause and its record go. |
| Disable or enable | Ends the pause too (as a disable ended a startup failure before): the package starts afresh. |
| Uninstall | The record goes with the package. An uninstall that cannot be recorded leaves it installed and still paused. |

A reload whose new code fails to start ([#11](https://github.com/hoangvu12/pane/issues/11))
is now one such pause: the Retry and diagnostics it offered are these, and
it holds across a restart. The earlier code is still not restored.

## When the extension runtime itself crashes

Added for [#17](https://github.com/hoangvu12/pane/issues/17) (US77, US78,
US80, T18, T19, G3; contributions, not a claim that the whole scenario or
gate passes). The [extension runtime](../CONTEXT.md) runs every
extension; it is a thread in Pane's process today (the ticket speaks of
killing the runtime process; whether it becomes one is pending the user's
decision, see [current decisions](current-decisions.md) Q39). A **runtime crash** is a panic of
that thread: a fault in Pane's host code or in Wasmtime, not a guest trap
(a trap is caught and counts as a crash of its package, above). Pane cannot
tell which extension, if any, caused it, so:

- **No extension is named or paused**, and no crash is counted towards
  pausing one. The status line says "Pane's extension runtime stopped
  unexpectedly and was started again; what was running was stopped and is
  not run again. Saved data is kept; details are in Manage extensions."
- **Every call the thread held stops**, running or queued, and answers
  "Extension runtime unavailable: it stopped before answering and was
  started again (or was not restarted, and why); Pane does not run this
  again by itself". None is sent again: an action that saved before its
  answer was lost stays done once, and running it again is the user's
  choice. Its guest instances, custom views and streams go with the
  thread; a custom view on screen closes, returning to its command; a
  command's list and a form stay open (their next call starts a fresh
  instance).
- **The native helpers it ran are ended** and reaped before anything
  else happens, since all of them were started by its guests
  ([helpers](helpers.md)); development builds run outside the runtime and
  are not affected.
- **Saved data is kept**: Pane writes extension data itself, never through
  the runtime thread's state.
- **Navigation and management keep working**: root search, Manage
  extensions, enabling and disabling, clearing a cache, uninstalling,
  deleting retained data and the hotkey and alias screens run no
  extension. Installing, updating and reloading check components on a
  checker thread of their own, which answers a check that panics and
  carries on.
- **Restarting is suppressed after a repeat.** Pane starts a fresh runtime
  thread (with a fresh engine) after a crash, unless the runtime crashed
  within 5 minutes before (`CRASH_WINDOW` in
  [`runtime/supervisor.rs`](../crates/pane-core/src/runtime/supervisor.rs),
  the same window as for pausing a package):
  then it stays stopped, and every extension call answers "it stopped after
  crashing and runs nothing until you restart it in Manage extensions".
- **Manage extensions** then starts with **Restart the extension runtime**
  (when Pane did not restart it) and **Why the extension runtime stopped**,
  whose screen says what happened and what Pane did, and shows the panic
  message ("Diagnostics"; the backtrace, if enabled, goes to standard error
  with the rest of the report). Restarting forgets earlier crashes, so the
  next one restarts it again by itself. The window redraws by itself when
  the crash is reported.

Faults are injected to check this, since no extension can crash the
runtime thread: `Runtime::inject(Fault::Crash)` panics the thread wherever
it waits (for the next request, or for a guest's clock, helper or
operation), `Fault::CrashBeforeAnswer { item }` panics it once the action
`item` has run, before its answer is sent (a lost response after a
completed side effect; other calls, such as root search's, are not
affected). The native smokes set `PANE_TEST_RUNTIME_FAULTS` to a file whose
appearance injects one (`crash`, or `crash-before-answer:count`). All of
this exists in debug builds only (`cfg(any(test, debug_assertions))`, which
the tests and smokes use): a release build has no fault types, hook or
environment variable.

Recovery relies on the panic **unwinding** to a `catch_unwind` on the
runtime thread: Pane refuses to build with `panic = "abort"`
(`compile_error!` in `runtime/supervisor.rs`).

**Taking over a lock.** The runtime's own locks ignore poisoning (one
`lock` helper in `runtime.rs`); what they guard is replaced as a whole, so
it is never half written. The launcher's state is different: the runtime
thread holds it while it notes a package's failure, and could panic half
way through pausing it. When the launcher finds its state poisoned, it
clears the poison and first resets it: every claim of a change in progress
is dropped (the panicked thread's would never be released; a change still
running elsewhere finishes, but another change to the same package is no
longer refused meanwhile), the listed pauses are made to agree with the
packages whose code is stopped (one stopped but not yet listed is listed as
paused, with Retry), and root search is shown afresh, closing any command,
form or custom view, with "Pane recovered from an internal error". The
installed packages, their records and extension data are not rebuilt:
they change only after their change is written.

Limits: a failure that ends Pane's whole process is not recovered: an
abort, a fault in native code, the system killing Pane, or **a panic in a
destructor while the thread unwinds from the first panic** (Rust then
aborts the process). The runtime is a thread in Pane's process today, not
a process of its own. The crash history is in memory only. A crash that loses the answer of an operation call loses the
caller's answer too; neither is sent again. A guest computing without
yielding holds the thread, so an injected crash waits for it to yield
(#18).

## Author example and tests

- The Rust settings sample's **Count** adds one to a count in its content
  and answers it: after a runtime crash lost its answer, the count shows
  it ran once and was not run again.
- The settings samples' **Crash** item
  ([Rust](../guests/sample-settings/src/lib.rs),
  [JavaScript](../guests/sample-settings-js/src/index.js),
  [TypeScript](../guests/sample-settings-ts/src/index.ts)) crashes on
  purpose: a Rust panic traps; in JavaScript and TypeScript, resolving an
  action with something other than a string does (throwing is an error the
  extension answers with).
- [`crates/pane-core/tests/pausing.rs`](../crates/pane-core/tests/pausing.rs):
  three crashes pause each language's sample, with its data kept, the pause
  held after a restart and Retry starting it; errors it answers with never
  pause it; an `Error` thrown from a form is an error in each language; the
  crash count starts afresh after a restart; another package keeps running;
  disabling, enabling or reloading ends the pause; a pause recorded for
  another version does not hold; a Retry without a runtime keeps the pause;
  a component that cannot load is paused at once; an uninstall that cannot
  be recorded keeps the pause; a reload that fails to start is paused across
  a restart; a root result provider that keeps crashing is paused and asked
  no more. [`aliases.rs`](../crates/pane-core/tests/aliases.rs): a command
  whose query ("crash") traps three times is paused, in Rust, JavaScript and
  TypeScript, and its alias row then explains the pause and runs nothing. Unit tests in `launcher/pausing.rs` cover the crash window with
  explicit times and crashes of code disabled meanwhile.
- [`crates/pane-core/tests/runtime_crash.rs`](../crates/pane-core/tests/runtime_crash.rs)
  (#17): with the settings and helper samples running, a crash ends the
  waiting helper (Pane lists none and its heartbeat stops), names no
  extension, pauses nothing, keeps saved data and restarts the runtime;
  Manage extensions, disable and uninstall work; a second crash soon after
  stops it until **Restart the extension runtime**; the settings sample's
  **Count**, whose answer a crash lost after it saved, is not run again.
  [`runtime.rs`](../crates/pane-core/src/runtime.rs) checks that a
  restarted thread never reuses a view id; `runtime/supervisor.rs` tests
  the restart window with explicit times.
  [`crates/pane/tests/runtime_crash.rs`](../crates/pane/tests/runtime_crash.rs):
  in the window, a crash closes the open custom view and redraws with the
  explanation; the details and Restart rows render and work. The native
  smokes' runtime-crash phase (frames 200 to 209) does the same with real
  key events.
- [`crates/pane-core/tests/operations.rs`](../crates/pane-core/tests/operations.rs):
  a target that keeps crashing is paused and its caller is not; a target
  stopped with its caller again and again is not paused.
- [`crates/pane-core/tests/hotkeys.rs`](../crates/pane-core/tests/hotkeys.rs):
  a paused command's hotkey stays registered and explains the pause.
- [`crates/pane/tests/install.rs`](../crates/pane/tests/install.rs): in the
  window, three crashes show the toast and the paused command's reason, and
  Retry in the extension list starts it again; the details of a reload that
  failed to start are rendered on their own screen.

## Limits

- The toast is the launcher's status line, replaced by the next action's
  outcome; the lasting status is in Manage extensions. When a crash of an
  operation's target pauses it, the caller's own answer (which reports the
  crash) is shown instead of the toast.
- A guest computing without yielding is not preempted, so it cannot crash
  or be paused before it yields (#18).
- The crash count is not kept across restarts; a package that crashes
  twice per session is never paused.
- `pane_js.py`'s generated entry, which applies the JS adapter, is not part
  of the prebuilt components' input digest (the adapter itself is): a change
  to that template alone needs `cargo xtask js-guests` by hand.
- Nothing here is platform-specific (it lives in `pane-core`); it has run on
  Linux, and runs in `cargo xtask ci` on Windows, macOS and Linux.
