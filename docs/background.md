# Background work: scheduled tasks and services

Added for [#47](https://github.com/hoangvu12/pane/issues/47) (US14, US45,
US57, US79; T02, T09, T17; G3; contributions, not a claim that the whole
scenario or gate passes). A command can declare a **scheduled task**
([glossary](../CONTEXT.md)): Pane runs it in the background every so
often, once the user turns its schedule on in Manage extensions, shows its
latest result there, and stops it when the user turns it off or the package
stops ([ADR 0022](adr/0022-run-background-work-beside-calls.md), proposed).
The choices below are **provisional**, pending the user's confirmation (see
[current decisions](current-decisions.md), Q15). A command can also declare
a **continuing service** ([#48](https://github.com/hoangvu12/pane/issues/48),
[below](#continuing-services)), which Pane keeps running once the user
starts it.

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

## Continuing services

Added for [#48](https://github.com/hoangvu12/pane/issues/48) (US14, US45,
US57, US79; T02, T09, T17; G3; contributions, not a claim that the whole
scenario or gate passes). A command can declare a **continuing service**
([glossary](../CONTEXT.md)): background work Pane keeps running while its
package's code runs, once the user starts it in Manage extensions, showing
the status it sets there, and stopping it when the user stops it or the
package's code stops. It is built on the same background work as scheduled
tasks ([ADR 0022](adr/0022-run-background-work-beside-calls.md), proposed);
its choices are **provisional** too (item 14 of
[current decisions](current-decisions.md)).

### Declaring a service

One flag on the command's entry in `pane.json`; anything but `true` or
`false` makes the manifest invalid:

```json
{ "id": "heartbeat", "title": "Heartbeat", "component": "heartbeat.wasm",
  "service": true }
```

The command's component also exports `pane:extension/service`
([`wit/background.wit`](../wit/background.wit)), whose one function,
`run-service(command)`, runs the service until it has nothing left to do,
normally a loop awaiting what it watches. It shows how it is doing with
the import `pane:extension/service-status`'s `set-status(text)`, which only
the service's own run may call (a command's call or a scheduled task is
refused, and so is a service Pane stopped). Pane checks the export at
install. Rust commands use `pane_guest::service` (`export!`, `Guest`,
`set_status`), JavaScript and TypeScript commands export `service` with
`"pane": { "service": true }` in `package.json` and import `setStatus` from
`"pane:extension/service-status@0.1.0"`, which the build links only into a
command whose bundle uses it
([author instructions](../guests/README.md#a-continuing-service)).

### Starting and stopping it, and its lifetime

- **Installing starts nothing.** Its **Service: <command>** row in Manage
  extensions says "Stopped · Start it to run in the background while Pane
  runs"; Enter starts it, and again stops it. The row is listed for an
  enabled package's command; a command not available on this system is
  listed as unavailable.
- **Started, it runs whenever its package's code may run**: at once, when
  Pane starts, and whenever the package's code starts again (enabled,
  reloaded, updated, retried after a pause), one run at a time. The choice
  is Pane's own record, `services.json` beside `installed.json`, by command
  id: `{ "version": 1, "services": ["<command id>"] }`. Stopping it forgets
  it; disabling the package keeps it.
- **Its status** is what its run set last, shown on the row while it runs
  ("Running · Beat 12"); it is kept in memory only, goes when the run ends,
  and a status arriving from a run that is no longer the service's is
  dropped. The row redraws as it changes.
- **Stopping it** (the user's Enter), or its package's code ending
  (disabled, reloaded, updated, paused, uninstalled), stops the run where
  it waits, with its instance, so nothing after its `await` runs, its answer
  is discarded and its status goes. A reload or update starts the new
  code's service at once; the old run is stopped first, so there is never
  more than one.

### How it ends on its own, and restarts

| End | Shown | What happens next |
| --- | --- | --- |
| It answers text | "Finished: <text> · Stop and start it to run it again" | Not started again until its package's code starts again (Pane starts, enable, reload, update, Retry) or the user stops and starts it. |
| It answers an error | "Failed: <error> · It starts again a minute after it ended" | An ordinary outcome: never counted against the package. Pane starts it again a minute (`RESTART_DELAY`) after it ended, on the launcher's clock, again and again. |
| It crashes (a trap) | "Crashed: <reason> · It starts again a minute after it ended" | Counts towards [pausing](pausing.md) the package exactly as any crash does: started again a minute later, the third crash within five minutes pauses the package; the row then says "Not running: <title> is paused after an error; …" and Retry starts it again at once. |
| The runtime thread crashes ([#17](pausing.md#when-the-extension-runtime-itself-crashes)) | "Failed: Extension runtime unavailable: …" | Pane names and pauses no package; the service starts again a minute later, on the restarted runtime. |

A component that cannot load or be instantiated fails to start, which
pauses the package at once, as for a call. A minute between restarts keeps
a failing service from spinning, and lets three crashes fall within the
five-minute window so that repeated crashes do pause it.

### Running long without being unresponsive

A service runs as long as Pane lets it, so it must never be judged by how
long it has run. The contract, which the watchdog of
[#18](https://github.com/hoangvu12/pane/issues/18) (CPU-time metering,
progress-based) is to follow ([ADR 0022](adr/0022-run-background-work-beside-calls.md)):

- A service **awaits between pieces of work** (a clock, a web request).
  While it waits it holds nothing: the runtime serves every other call and
  background run meanwhile, and none of that time is its.
- What counts is the **CPU time of each slice** it computes between two
  awaits. A slice that computes too long without yielding is a hang of the
  package, whether the slice is a call's, a task's or a service's; a
  service that runs for days, awaiting between beats, is never one.
- Until #18 lands, a slice computing without yielding holds the runtime
  thread, and every call behind it, as a call's would.

### Author example

The service samples ([Rust](../guests/sample-service/src/lib.rs),
[JavaScript](../guests/sample-service-js/src/index.js),
[TypeScript](../guests/sample-service-ts/src/index.ts)) declare Heartbeat,
whose service beats every second: each beat adds one to a count kept in the
package's content and sets the status "Beat N". Its one task is that timer,
awaited in a loop. Opened, the command shows the count and chooses how the
service behaves at its next beat: keep beating, stop with an error, crash
on purpose or finish.

### Checks

- [`crates/pane-core/tests/services.rs`](../crates/pane-core/tests/services.rs),
  through the launcher's public interface on a `ManualClock`, with the
  samples in Rust, JavaScript and TypeScript: installing starts nothing
  (not even ten restart delays later); starting it (Enter on its row) runs
  it and the row shows "Running · Beat N", which follows its beats; the
  record lists it; stopping it releases its instance (the runtime lists no
  background work) and the record forgets it; started, it starts again when
  Pane starts again; disabling stops it and releases its instance, nothing
  restarts it while disabled, enabling starts it again once; reloading
  replaces the old run with one of the new code; uninstalling stops,
  releases and forgets it; an error is shown and restarts it a minute later
  (not at 59 seconds), again and again without pausing; three crashes pause
  the package, a paused service does not start, Retry starts it; a finished
  service waits for its package's code to start again; a running service
  holds back no command of another package and starts none; a service lost
  to a runtime crash starts again a minute later; a launcher without
  background work explains it.
- Runtime test (`runtime.rs`): a service's statuses reach its sink, a call
  is served while it waits, a disable stops it and releases its instance,
  and it answers how it finished or its error. `packages.rs` checks the
  declaration.
- Native GUI smokes on all three systems (frames 330 to 335, data folder
  `service-data`): install the Rust sample (nothing runs), start its
  service in Manage extensions (the row shows "Running · Beat N", the
  record holds it, the count grows), disable the package (for three beats'
  time the count does not move), enable it (started again at once, the
  count grows), and stop it (the count stops; the record forgets it). See
  the [Linux](platforms/linux.md#continuing-services-48),
  [macOS](platforms/macos.md#continuing-services-48) and
  [Windows](platforms/windows.md#continuing-services-48) notes for where it
  has run.

### Limits of services

- One flag, no policy: no start at install, no restart choice, no
  back-off beyond the fixed minute, no limit on how many services run.
- The status is text, shown in Manage extensions only (no notification,
  menu bar item or root search row), and not kept across restarts.
- A service cannot call other packages' operations, like a task.
- Its instance is its own: what it keeps in memory is lost when it stops;
  what must survive goes to extension data.
- An update or reload always restarts it, even when nothing it uses changed
  (the [safe activation boundary](https://github.com/hoangvu12/pane/issues/49)
  of later updates is #49).
- Nothing here is platform-specific (it lives in `pane-core`); it has run
  natively on Linux X11 only.
