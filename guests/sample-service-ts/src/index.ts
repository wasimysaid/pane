// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Pane's service sample in TypeScript: Heartbeat, a command with a
// continuing service. Once the user starts it in Manage extensions, Pane
// keeps it running in the background while its package's code runs: it
// beats every second, counting each beat in the package's content and
// showing "Beat N" as its status. Pane starts it again whenever the
// package's code starts again (Pane starts, the package is enabled,
// reloaded, updated or retried) and stops it where it waits when the package
// is disabled, reloaded, updated, paused or uninstalled, or the user stops
// it. Opened, the command shows the count and chooses how the service
// behaves at its next beat: keep beating, stop with an error (an ordinary
// outcome; Pane starts it again a minute later), crash on purpose (resolving
// with something other than a string; crashing again and again pauses the
// package) or finish (not started again until the package's code starts
// again). Items, answers and errors match the Rust service sample
// (guests/sample-service) and the JavaScript one.
import type { Command, Item, Service } from "@pane/extension";
import * as content from "pane:extension/content@0.1.0";
import { setStatus } from "pane:extension/service-status@0.1.0";
import * as settings from "pane:extension/settings@0.1.0";
import { waitFor } from "wasi:clocks/monotonic-clock@0.3.0";

/** The content key holding how many beats the service counted. */
const BEATS = "beats";
/** The settings key holding how the service behaves at its next beat. */
const MODE = "beat-mode";
/** How long the service waits between beats, in nanoseconds. */
const BEAT = 1_000_000_000;

/** The beats the service counted. */
function beats(): number {
  const count = content.get(BEATS);
  if (count === null) return 0;
  const parsed = Number(count);
  if (!Number.isInteger(parsed)) throw new Error("the count is not a number");
  return parsed;
}

const item = (id: string, title: string, subtitle: string): Item => ({ id, title, subtitle });

const CHOSEN: Record<string, string> = {
  beat: "Heartbeat keeps beating",
  fail: "Heartbeat stops with an error at its next beat",
  crash: "Heartbeat crashes at its next beat",
  finish: "Heartbeat finishes at its next beat",
};

export const command: Command = {
  async getView() {
    const mode = settings.get(MODE) ?? "beat";
    const chosen = (id: string) => (mode === id ? " (chosen)" : "");
    return {
      title: "Heartbeat: a continuing service",
      items: [
        item(
          "count",
          `Beat ${beats()} times`,
          "Start it in Manage extensions to beat every second",
        ),
        item("beat", `Keep beating${chosen("beat")}`, "It runs on, counting one beat a second"),
        item(
          "fail",
          `Stop with an error${chosen("fail")}`,
          "It fails at its next beat; Pane starts it again a minute later",
        ),
        item(
          "crash",
          `Crash on purpose${chosen("crash")}`,
          "It crashes at its next beat; crashing again and again pauses it",
        ),
        item(
          "finish",
          `Finish${chosen("finish")}`,
          "It finishes at its next beat, until its package starts again",
        ),
      ],
    };
  },
  async runAction(itemId) {
    if (itemId === "count") return `Beat ${beats()} times`;
    const answer = CHOSEN[itemId];
    if (answer === undefined) throw new Error(`unknown item: ${itemId}`);
    settings.set(MODE, itemId);
    return answer;
  },
  async submitForm() {
    throw new Error("Heartbeat has no forms");
  },
  async openView() {
    throw new Error("Heartbeat has no custom views");
  },
};

export const service: Service = {
  async runService(command) {
    if (command !== "heartbeat") throw new Error(`unknown command: ${command}`);
    for (;;) {
      switch (settings.get(MODE)) {
        case "fail":
          throw new Error("Heartbeat stops with an error, to show how a failed service looks");
        case "crash":
          // Resolving with something other than a string is a crash, unlike
          // throwing, which is an error the extension answers with.
          return undefined as unknown as string;
        case "finish":
          return `Finished after ${beats()} beats`;
      }
      const count = beats() + 1;
      content.set(BEATS, `${count}`);
      setStatus(`Beat ${count}`);
      // The service suspends here; if Pane stops it meanwhile, nothing
      // after this line runs, and no beat is counted.
      await waitFor(BEAT);
    }
  },
};
