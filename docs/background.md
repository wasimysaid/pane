# Background work: scheduled tasks

Added for [#47](https://github.com/hoangvu12/pane/issues/47) (US14, US45,
US57, US79; T02, T09, T17; G3; contributions, not a claim that the whole
scenario or gate passes). A command can declare a **scheduled task**
([glossary](../CONTEXT.md)): Pane runs it in the background every so
often, once the user turns its schedule on in Manage extensions, shows its
latest result there, and stops it when the user turns it off or the package
stops ([ADR 0022](adr/0022-run-background-work-beside-calls.md), proposed).
The choices below are **provisional**, pending the user's confirmation (see
[current decisions](current-decisions.md), Q15).

## The declaration

One kind of schedule, with a bounded policy: every so many minutes, from 1
to 1440 (a day), on the command's entry in `pane.json`:

```json
{ "id": "ticks", "title": "Ticks", "component": "ticks.wasm",
  "schedule": { "everyMinutes": 15 } }
```

Anything else under `schedule` (a time of day, a cron line, another unit, a
string) makes the manifest invalid rather than being ignored. The command's
component also exports `pane:extension/scheduled-task`
([`wit/background.wit`](../wit/background.wit)), whose one function,
`run-task(command)`, runs the task once and answers text or an error; Pane
checks the export at install without running any code. Rust commands export
it through `pane_guest::scheduled`, JavaScript and TypeScript commands by
exporting `scheduledTask` with `"pane": { "scheduledTask": true }` in
`package.json` ([author instructions](../guests/README.md#a-scheduled-task)).

## Turning it on, and when it runs

- **Installing schedules nothing.** A scheduled task runs only after the
  user turns its schedule on, in its **Schedule: <command>** row of Manage
  extensions ("Off · Run it in the background every 15 minutes"); Enter
  turns it on, and again off. The row is listed for an enabled package's
  command, like its hotkey row; a command not available on this system is
  listed as unavailable.
- **Turning it on runs it at once**, then one interval after each run
  began. The choice is Pane's own record, `schedules.json` beside
  `installed.json`, by command id (not extension data), with when the task
  last ran and how its last run ended:
  `{ "version": 1, "tasks": { "<command id>": { "lastRun": 1800000000, "outcome": "answered", "text": "Ticked 3 times" } } }`.
  Turning it off forgets both; turning it on again runs it at once.
- **It runs only while its package's code may run**: the package enabled
  and not paused, not being reloaded, updated or uninstalled, the command
  available on this system, and Pane running. Disabling the package keeps
  the choice; enabling it again resumes the schedule.
- **What fell due meanwhile runs once.** When Pane starts, or the package is
  enabled or retried again, a task whose run fell due while it could not
  run is run at once, a single time however many intervals passed; the next
  interval counts from that run. A last run recorded in the future (the
  system's time went back) counts as due.
- **One run at a time.** A run still going when the next falls due is not
  doubled: the next is due one interval after the run in progress began.
- Times are the system's wall clock, as recorded across restarts. The
  launcher asks its clock (`pane_core::clock::Clock`) rather than the
  system, so the tests move a `ManualClock` by hand instead of waiting;
  the system clock is looked at again at least every 30 seconds while Pane
  waits, so a change of the time (or the computer sleeping) delays a run by
  no more than that.

## Where it runs, and what stops it

A run is **background work**, not a call: the runtime starts it in an
instance of its own (never the command's own instance, which the command's
calls keep using), drops that instance when the run ends, and serves the
calls queued meanwhile while the run waits on a clock or a web request. So
a run waiting ten seconds holds back no command the user opens, of its
package or another, and starts no other package
([ADR 0022](adr/0022-run-background-work-beside-calls.md)). The window's
thread never waits for it: a thread of the launcher's own (the driver)
starts what is due and waits for it.

The run belongs to its package's [generation](generations.md), as a call
does:

| Event | What happens to a run in progress |
| --- | --- |
| The package is disabled, reloaded, updated, paused or uninstalled | Its generation ends: the run is stopped where it waits, with its instance, so nothing after its `await` runs; an answer completing anyway is discarded. The row says "Last run stopped: <title> was disabled" (or reloaded or updated, paused, uninstalled) until the next run ends; the recorded result stays the one before. |
| The user turns the schedule off | The run is stopped the same way, and nothing of it is shown. |
| The runtime thread crashes ([#17](pausing.md#when-the-extension-runtime-itself-crashes)) | The run is lost with the thread, is not made again by itself, and is shown as a failure ("Extension runtime unavailable: …"); the next run is due one interval after it began. |

A reloaded or updated package's new code does **not** run at once to make
up for the run the reload stopped: its next run is due one interval after
the stopped one began. Enabling a package again does not rerun a run its
disable stopped either. So a stop never causes a second activation.

## Results, errors and crashes

- **An answer** is the task's latest result ("On · Every 15 minutes · Last
  run: Ticked 3 times"), recorded.
- **An error the task answers with** is its latest failure ("Last run
  failed: …"), recorded; the schedule goes on, and it never counts against
  the package: an error is an ordinary outcome, as for any call.
- **A crash** (a trap) is its latest outcome ("Last run crashed: …") and
  counts towards [pausing](pausing.md) the package exactly as any crash
  does: the third crash within five minutes pauses it, whatever mix of
  calls and runs crashed. While the package is paused the row says "Not
  running: <title> is paused after an error; …" and no run is due; Retry
  resumes the schedule, and what fell due meanwhile runs once, at once.
- A component that cannot load or be instantiated fails to start, which
  pauses the package at once, as it would for a call.
- No status line appears for a run: the lasting status is the row in Manage
  extensions, which redraws when a run ends.

## Author example

The background samples ([Rust](../guests/sample-background/src/lib.rs),
[JavaScript](../guests/sample-background-js/src/index.js),
[TypeScript](../guests/sample-background-ts/src/index.ts)) declare Ticks,
every minute. Each run adds one to a count kept in the package's content
and answers "Ticked N times". Opened, the command shows the count and
chooses how the next runs behave: answer normally, answer with an error,
crash on purpose, or wait 10 seconds (noting "started" before and
"finished" after the wait, so a stopped run is visible in its data).

## Checks

- [`crates/pane-core/tests/scheduled.rs`](../crates/pane-core/tests/scheduled.rs),
  through the launcher's public interface on a `ManualClock`, with the
  samples in Rust, JavaScript and TypeScript: installing runs nothing;
  turning the schedule on (Enter on its row) runs it at once; it runs again
  at a minute and not at 59 seconds; ten minutes at once make one run;
  turned off, nothing runs and the record forgets it; the schedule holds
  across a restart, a restart before it is due runs nothing until it is,
  an hour without Pane makes one run, and a last run in the future is due;
  errors are shown and the schedule goes on without pausing; three crashes
  pause the package, a paused task does not run, and Retry runs it once;
  disabling, reloading, turning off and uninstalling while a run waits stop
  it (the runtime lists no background work, "finished" is never noted, the
  stop is shown, and neither enabling nor the reloaded code runs it again
  before its time); a waiting run holds back no command of another package
  and starts none; a run lost to a runtime crash is shown as failed and not
  made again; a launcher without background work explains it.
- Runtime tests (`runtime.rs`): a waiting task lets a call of the same
  command answer at once, in the command's own instance, and stops with its
  generation; a task answers, errs, crashes (reported to the health report)
  or is stopped by its caller. `packages.rs` checks the declaration's
  bounds; `clock.rs` the manual clock.
- Native GUI smokes on all three systems (frames 320 to 327, data folder
  `schedule-data`): install the Rust sample (nothing runs), turn its
  schedule on in Manage extensions (it runs at once and the row shows the
  answer; the record holds it), choose "Wait 10 seconds in each run", turn
  it off and on again (a run waits), disable the package while it waits
  (after ten more seconds, nothing was noted as finished or counted), enable
  it again (the row says the run stopped, and it is not run again), and turn
  it off (the record forgets it). See the
  [Linux](platforms/linux.md#scheduled-tasks-47),
  [macOS](platforms/macos.md#scheduled-tasks-47) and
  [Windows](platforms/windows.md#scheduled-tasks-47) notes for where it has
  run.

## Limits

- One kind of schedule: no time of day, calendar, cron, jitter or
  per-user interval. The interval is the author's; the user can only turn
  it on or off.
- A task's code cannot call other packages' operations: they are served
  only in a call's own frame, so a run's call is refused
  ("operations can only be called while serving a Pane call").
- Each run starts a fresh instance (memory kept between runs is lost; keep
  state in extension data), which costs an instantiation per run.
- A run is served on the one runtime thread: while it computes without
  yielding it holds every other call, and a run that never yields holds
  them until its package stops (hangs and time limits are
  [#18](https://github.com/hoangvu12/pane/issues/18); see
  [ADR 0022](adr/0022-run-background-work-beside-calls.md) for how
  background work is meant to meet its watchdog).
- A crash every run of a task scheduled further apart than the crash window
  (a crash every 15 minutes) never adds up to a pause: each is shown on the
  row, and the schedule goes on.
- A run is not retried when it fails; the next is due one interval later.
- A task lost to Pane quitting is not recorded as stopped: at the next
  start it is simply due, and runs once.
- Nothing here is platform-specific (it lives in `pane-core`); it has run
  natively on Linux X11 only.
