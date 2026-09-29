# Generations: stopping pending calls

Added for [#14](https://github.com/hoangvu12/pane/issues/14) (US35, US54,
US57, US64, T06, T09, G2, G3; contributions, not a claim that the whole
scenario or gate passes). Disabling, reloading or updating an extension
stops the calls into it that are still pending, and their late results never
reach the screen or the extension's data. This page is the ownership and
cancellation model later lifecycle work builds on: native helpers (#15),
pausing a broken extension (#16), recovering from a guest that stops
responding (#18), uninstall (#40) and cancelling a pending search (#29, #30).

## The model

A **generation** ([glossary](../CONTEXT.md)) is one run of an installed
package's code: it begins when the package is installed, enabled or Pane
starts, and ends when the package is disabled, [paused](pausing.md) after it
failed, or its code is replaced by a reload or an update (a reload's or update's new code runs in a new
generation). A disabled package stays in an ended generation until it is
enabled again. Commands built into Pane have no generation: they run as long
as Pane.

Every guest call has two owners, with different powers:

| Owner | What it decides | Examples |
| --- | --- | --- |
| The package's **generation**, current when the call was asked for | Whether the call may run at all. When it ends, the call is **stopped**. | Opening a command, running an action, submitting a form, opening a custom view, a view event, computing root results, serving an operation |
| The **screen, view or search** the call answers | Whether its answer is **shown**. Leaving it only **discards** the answer; the call is not stopped. | Back from a command that is opening; a newer query; a newer view event's frame on screen |

- The launcher takes the generation when the user acts (pressing Enter,
  submitting), not when the returned future first runs, so a call asked for
  before a disable is never started for the package's new state.
- An **operation call** belongs to the target package's generation and,
  through the call chain, to every caller's: when any generation in the
  chain ends, the calls from it inward stop. A target stopped while serving
  a call answers its caller at once, with `disabled` or, for a reload or
  update, `unavailable` ("Package b was reloaded or updated while serving
  the call; call it again"); the caller carries on. A caller stopped while
  it waits stops the operation it waits for too.
- Navigation does not stop calls: an action may be doing what the user
  asked, such as saving, and leaving its screen is not a request to stop
  it. The one exception, since [#29](files.md#cancelling-a-pending-search),
  is root search's own calls for computed results: the search is a
  stopping owner of them, so a newer query or leaving root search cancels
  those pending (`CallError::Cancelled`), exactly as an ended generation
  stops them, and it is not a failure for pausing.

## What stopping does

The runtime serves guest calls one at a time on its own thread. When a
generation ends:

1. **Queued calls** of it (asked for but not started) are not started; they
   answer "The extension is disabled" or "The extension was reloaded or
   updated while this was running; try again", and the launcher shows
   nothing of them.
2. **A call waiting inside the guest** (on a clock, an operation call, any
   async import) is stopped the next time the runtime thread looks, which
   is immediately, since it is idle while the guest waits: the call's
   future is dropped and so is the guest instance, with everything its
   Wasmtime store holds (the abandoned task, its host tasks, streams,
   futures, custom views and resources). The instance must go: Wasmtime 49
   keeps a task whose host future was dropped in the store and would resume
   it on the next call, and it has no host-side `task.cancel` for an export
   the host called. So nothing after the guest's `await` runs, and a
   package serving an operation for a stopped caller loses its instance too
   (the next call starts it afresh, as after a crash).
3. **A result that completes anyway** (the guest returned in the same turn
   the generation ended) is discarded the same way.
4. **Host imports refuse the stopped code**, so it starts no host work of
   any kind: reading or saving any kind of extension data ("this code of
   the extension was replaced by a reload or an update; its settings are
   kept unchanged", or "the extension is disabled; …"), calling operations
   (`refused`), listing or opening applications, and starting a native
   helper ([helpers](helpers.md)). This matters only for
   code still running because it did not yield (below): code stopped at an
   `await` never runs again.
5. **Other packages keep running**, with their instances and open views.
6. Component checks (install, update, reload) run on a checker thread of
   their own, one at a time, so a reload's check does not wait behind the
   call it is about to stop.

The launcher then shows what the disable, reload or update says ("Disabled
<title>", "Reloaded <title>", "Updated <title> to <version>"); a command,
form or view of the package that was open has closed, and no answer of the
stopped code appears, on the old screen or on the new code's.

## What stopping costs

Stopping a call drops its whole instance, never just the call: Wasmtime 49
cannot cancel a guest task the host called, and a dropped call's task would
resume in the store. So:

- **The instance's in-memory state is lost** with it: an open view of the
  same instance, caches kept in guest memory, and so on. For disable,
  reload and update that is intended, since the code stops anyway. Later
  owners that stop calls while the package keeps running pay this cost
  too: cancelling a search's pending provider calls (#29) drops the
  provider's instance, losing what it keeps in memory between queries (the
  Files extension keeps nothing there).
  A command cancelling its own [native helper](helpers.md) run (#15) needs
  no such owner: the guest drops the run, Wasmtime cancels the host task
  and the process ends, while the instance stays. Such owners may prefer
  discarding answers to
  stopping calls, or waiting for component-model cancellation of a task
  the host called (`task.cancel` from the host) in a later Wasmtime.
- **A package serving an operation for a stopped caller restarts fresh**
  on its next call, although it was not disabled, as after a crash. It is
  not a failure of that package: [pausing](pausing.md) (#16) counts only
  crashes and failures to start, never a stopped call.

## What stopping cannot do yet

- **A guest computing without yielding cannot be preempted.** The runtime
  thread is inside the guest until it returns or awaits; the generation is
  checked only then, so a busy loop holds the thread, and every later call
  of every extension waits behind it. What such code does after the end
  (saving data, calling operations) is refused, and its result discarded,
  but it runs until it yields. Preempting it needs Wasmtime's epoch
  interruption (`Config::epoch_interruption` with a ticker thread and
  `Store::epoch_deadline_async_yield_and_update` or a trap) or fuel
  (`Config::consume_fuel`); both add per-call overhead, and choosing
  budgets, timeouts and what the user sees is
  [#18](https://github.com/hoangvu12/pane/issues/18).
- **No timeouts, and no user cancellation** of an action: a call ends when
  the guest answers, or when its package's generation ends.
- **External side effects are not undone**: what the guest did before the
  stop (a file written through WASI, a request sent) stays done; only what
  it would have done afterwards is prevented. Data it saved before the stop
  is kept (the sample's "started").
- **Native helper processes** ([helpers](helpers.md), #15) belong to their
  package's generation and are ended with it, by a thread that watches the
  generation itself, so even while the runtime thread is busy; they also
  end with the call that started them. Processes a helper starts itself are
  not stopped. Nothing in this model assumes one operating system: it lives
  in the runtime and the launcher, with no platform adapter.
- **Background work** belongs to a generation the same way, since
  [#47](background.md): a [scheduled task](background.md)'s run is started
  in an instance of its own for the generation current when it falls due,
  is stopped with its instance when that generation ends (or the user turns
  its schedule off), and its answer is then discarded;
  `Runtime::background_running` lists the runs in progress, which is how
  the tests see that none survives a disable or reload.
- Measured cleanup is what the runtime reports (`Runtime::running`,
  `Runtime::view_count`); memory returned to the operating system after a
  dropped store is not measured.

## Examples and tests

- The operations samples ([Rust](../guests/sample-operations/src/lib.rs),
  [JavaScript](../guests/sample-operations-js/src/index.js),
  [TypeScript](../guests/sample-operations-ts/src/index.ts)) publish
  `wait`, which saves `waiting` as "started", waits ten seconds and saves
  "finished"; their "Wait in another extension" item calls it in another
  package.
- The settings samples' **Save after waiting**
  ([Rust](../guests/sample-settings/src/lib.rs),
  [JavaScript](../guests/sample-settings-js/src/index.js),
  [TypeScript](../guests/sample-settings-ts/src/index.ts)) save "started",
  wait ten seconds with `wasi:clocks/monotonic-clock.wait-for`, then save
  "finished". Disabling or reloading the package meanwhile ends the call at
  once and "finished" is never saved.
- [`crates/pane-core/tests/stopping.rs`](../crates/pane-core/tests/stopping.rs)
  drives them in all three languages through the launcher: disabling,
  reloading and updating while the call waits (it ends in well under the
  wait, its answer is not shown, "finished" is not saved, and the new code
  runs), three disable and reload cycles each leaving only the expected
  instances running and no views, and a call queued behind a stopped one
  that belonged to the replaced code.
- [`crates/pane-core/tests/operations.rs`](../crates/pane-core/tests/operations.rs)
  stops a chain from both ends with the operations fixture's `wait`:
  disabling or reloading the target while it serves (the caller is told at
  once), and disabling the caller (the target's instance goes, the target
  itself stays enabled and serves the next call); and with the samples'
  `wait` from Rust to JavaScript, JavaScript to TypeScript and TypeScript
  to Rust, from both ends. The fixture's spinning items compute without
  yielding while the test disables or reloads `a`: what they try
  afterwards (saving, calling `b`) is refused, and their answer, or their
  error, completing in the same turn is discarded.
- Runtime tests (`runtime.rs`): stopping a call whose instance holds a
  stream open to the host (stdout), the pending future of that write and
  an open custom view releases them all (the faulty fixture's `hold`); and
  a stale call of an ended generation, served after the next generation
  opened a view at the same component, is refused without touching it.
- These run in `cargo xtask ci`, which CI runs on Windows, macOS and Linux;
  when this was written they had run on Linux only. The earlier 20 sequential calls in one QuickJS
  instance are not concurrency evidence; the overlapping calls here are
  queued calls behind a pending one, and a chain's calls, still served one
  at a time.
