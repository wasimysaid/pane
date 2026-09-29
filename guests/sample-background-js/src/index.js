// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Pane's background sample in JavaScript: Ticks, a command with a scheduled
// task. Once the user turns its schedule on in Manage extensions, Pane runs
// the task in the background every minute: each run adds one to a count kept
// in the package's content and answers "Ticked N times". Opened, the command
// shows the count and chooses how the next runs behave: answer normally,
// answer with an error (an ordinary outcome; the schedule goes on), crash on
// purpose (resolving with something other than a string; crashing again and
// again pauses the package) or wait 10 seconds (disabling or reloading the
// package, or turning the schedule off, meanwhile stops the run where it
// waits, so it never notes "finished"). Items, answers and errors match the
// Rust background sample (guests/sample-background) and the TypeScript one.
// Each run starts in an instance of its own: the count lives in content.
import * as content from "pane:extension/content@0.1.0";
import * as settings from "pane:extension/settings@0.1.0";
import { waitFor } from "wasi:clocks/monotonic-clock@0.3.0";

/** The content key holding how many runs counted. */
const TICKS = "ticks";
/** The settings key holding how the next runs behave. */
const MODE = "tick-mode";
/** The content key where a waiting run notes how far it got. */
const WAITED = "tick-wait";
/** How long a waiting run waits, in nanoseconds. */
const WAIT = 10_000_000_000;

/** The count the runs kept. */
function ticks() {
  const count = content.get(TICKS);
  if (count == null) return 0;
  const parsed = Number(count);
  if (!Number.isInteger(parsed)) throw new Error("the count is not a number");
  return parsed;
}

/** Adds one to the count, and answers the new count. */
function tick() {
  const count = ticks() + 1;
  content.set(TICKS, `${count}`);
  return `Ticked ${count} times`;
}

/**
 * @param {string} id
 * @param {string} title
 * @param {string} subtitle
 * @returns {import("@pane/extension").Item}
 */
const item = (id, title, subtitle) => ({ id, title, subtitle });

/** @type {Record<string, string>} */
const CHOSEN = {
  answer: "The next runs count",
  fail: "The next runs answer an error",
  crash: "The next runs crash",
  wait: "The next runs wait 10 seconds, then count",
};

/** @type {import("@pane/extension").Command} */
export const command = {
  async getView() {
    const mode = settings.get(MODE) ?? "answer";
    /** @param {string} id */
    const chosen = (id) => (mode === id ? " (chosen)" : "");
    return {
      title: "Ticks: a scheduled task",
      items: [
        item(
          "count",
          `Ticked ${ticks()} times`,
          "Turn its schedule on in Manage extensions to count every minute",
        ),
        item("answer", `Answer normally${chosen("answer")}`, "Each run counts one more"),
        item(
          "fail",
          `Answer with an error${chosen("fail")}`,
          "Each run answers an error; the schedule goes on",
        ),
        item(
          "crash",
          `Crash on purpose${chosen("crash")}`,
          "Each run crashes; crashing again and again pauses it",
        ),
        item(
          "wait",
          `Wait 10 seconds in each run${chosen("wait")}`,
          "Disabling or reloading it meanwhile stops the run",
        ),
      ],
    };
  },
  async runAction(itemId) {
    if (itemId === "count") return `Ticked ${ticks()} times`;
    const answer = CHOSEN[itemId];
    if (answer === undefined) throw new Error(`unknown item: ${itemId}`);
    settings.set(MODE, itemId);
    return answer;
  },
  async submitForm() {
    throw new Error("Ticks has no forms");
  },
  async openView() {
    throw new Error("Ticks has no custom views");
  },
};

/** @type {import("@pane/extension").ScheduledTask} */
export const scheduledTask = {
  async runTask(command) {
    if (command !== "ticks") throw new Error(`unknown command: ${command}`);
    switch (settings.get(MODE)) {
      case "fail":
        throw new Error("Ticks refuses to count, to show how a failed run looks");
      case "crash":
        // Resolving with something other than a string is a crash, unlike
        // throwing, which is an error the extension answers with.
        return /** @type {string} */ (/** @type {unknown} */ (undefined));
      case "wait":
        content.set(WAITED, "started");
        // The run suspends here; if Pane stops it meanwhile, nothing after
        // this line runs.
        await waitFor(WAIT);
        content.set(WAITED, "finished");
        return tick();
      default:
        return tick();
    }
  },
};
