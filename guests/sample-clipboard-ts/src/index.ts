// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Pane's clipboard history sample in TypeScript: the same contract as the
// Clipboard History default extension (guests/clipboard-history, Rust) and
// the JavaScript sample. Pane's host watches the clipboard and keeps the
// text the user copies for this package once they turn it on here, through
// `pane:extension/clipboard-history` (imported because package.json sets
// `"pane": { "clipboardHistory": true }`); the command only shows the
// history and the user's controls: turn on, pause, resume, turn off, exclude
// a program, clear, and Enter on an item copies it again.
import * as history from "pane:extension/clipboard-history@0.1.0";
import type { Capture, Entry, HistoryStatus } from "pane:extension/clipboard-history@0.1.0";
import type { Command, CustomView, FieldValue, Item, View } from "@pane/extension";

/** The longest title of a kept item, in characters. */
const TITLE_CHARS = 80;

const CAPTURES: Record<string, [Capture, string]> = {
  "turn-on": ["on", "Clipboard history is on"],
  pause: ["paused", "Clipboard history is paused"],
  resume: ["on", "Clipboard history is on again"],
  "turn-off": ["off", "Clipboard history is off"],
};
const INCLUDE = "include:";
const ENTRY = "entry:";

/** The reason a host function failed with. */
const reason = (error: unknown): string => String((error as { payload?: unknown }).payload);

/** A host function's result, or an `Error` with the reason it failed. */
function host<T>(call: () => T): T {
  try {
    return call();
  } catch (error) {
    throw new Error(reason(error));
  }
}

const item = (id: string, title: string, subtitle: string): Item => ({ id, title, subtitle });

const plural = (count: number, one: string, many: string): string =>
  count === 1 ? `1 ${one}` : `${count} ${many}`;

function toggle(status: HistoryStatus): Item {
  const kept = plural(status.items, "item", "items");
  const [id, title, subtitle]: [string, string, string] =
    status.capture === "off"
      ? [
          "turn-on",
          "Turn on clipboard history",
          "Off · Pane keeps nothing you copy until you turn it on. Once on, it keeps the text you " +
            "copy on this computer; nothing is sent anywhere",
        ]
      : status.capture === "on"
        ? ["pause", "Pause clipboard history", `On · ${kept} kept · Text you copy is kept on this computer`]
        : [
            "resume",
            "Resume clipboard history",
            `Paused · ${kept} kept · Nothing you copy is kept until you resume`,
          ];
  return item(id, title, status.problem ? `${status.problem} · ${subtitle}` : subtitle);
}

/** The first line of `text` with content, trimmed and at most `TITLE_CHARS` long. */
function titleOf(text: string): string {
  const line = text.split(/\r?\n/).map((line) => line.trim()).find((line) => line !== "") ?? "";
  const chars = [...line];
  return chars.length <= TITLE_CHARS ? line : chars.slice(0, TITLE_CHARS - 1).join("") + "…";
}

function age(seconds: number): string {
  if (seconds < 60) return "just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} h ago`;
  if (seconds < 2 * 86400) return "1 day ago";
  return `${Math.floor(seconds / 86400)} days ago`;
}

function entryItem(entry: Entry): Item {
  const about = [age(entry.ageSeconds)];
  if (entry.source) about.push(`from ${entry.source}`);
  const lines = entry.text.split(/\r?\n/).length - (entry.text.endsWith("\n") ? 1 : 0);
  if (lines > 1) about.push(`${lines} lines`);
  about.push("Enter copies it");
  return item(`${ENTRY}${entry.id}`, titleOf(entry.text), about.join(" · "));
}

async function getView(): Promise<View> {
  const status = host(history.status);
  const items: Item[] = [toggle(status)];
  if (status.capture !== "off") {
    items.push(
      item(
        "turn-off",
        "Turn off clipboard history",
        "Stops keeping what you copy; the kept items stay until you clear them",
      ),
    );
  }
  const excluded = status.excluded.length === 0 ? "None excluded" : `${status.excluded.length} excluded`;
  items.push({
    ...item("exclude", "Exclude a program", `Text copied from it is never kept · ${excluded}`),
    form: {
      title: "Exclude a program",
      fields: [
        {
          id: "program",
          label: "Program file name",
          kind: { tag: "text", val: { placeholder: "KeePass.exe" } },
        },
      ],
      submitLabel: "Exclude",
    },
  });
  for (const program of status.excluded) {
    items.push(
      item(`${INCLUDE}${program}`, `Stop excluding ${program}`, `Text copied from ${program} is not kept`),
    );
  }
  const entries = host(history.entries);
  if (entries.length > 0) {
    items.push(
      item(
        "clear",
        "Clear clipboard history",
        `Deletes the ${plural(status.items, "item", "items")} kept; whether history is kept does not change`,
      ),
    );
  }
  items.push(...entries.map(entryItem));
  if (entries.length === 0 && status.capture === "on") {
    items.push(item("empty", "Nothing kept yet", "Text you copy from now on is listed here"));
  }
  return { title: "Clipboard history (TypeScript)", items };
}

async function runAction(itemId: string): Promise<string> {
  const wanted = CAPTURES[itemId];
  if (wanted) {
    host(() => history.setCapture(wanted[0]));
    return wanted[1];
  }
  if (itemId === "clear") {
    return `Deleted ${plural(host(history.clear), "kept item", "kept items")}`;
  }
  if (itemId === "empty") return "Nothing is kept yet";
  if (itemId.startsWith(INCLUDE)) {
    const program = itemId.slice(INCLUDE.length);
    const excluded = host(history.status).excluded.filter((excluded) => excluded !== program);
    host(() => history.setExcluded(excluded));
    return `Text copied from ${program} is kept again`;
  }
  if (itemId.startsWith(ENTRY)) {
    host(() => history.copy(itemId.slice(ENTRY.length)));
    return "Copied to the clipboard";
  }
  throw new Error(`unknown item: ${itemId}`);
}

async function submitForm(itemId: string, values: FieldValue[]): Promise<string> {
  if (itemId !== "exclude") throw { message: `unknown form: ${itemId}` };
  const program = (values.find((value) => value.id === "program")?.value ?? "").trim();
  const excluded = host(history.status).excluded;
  try {
    history.setExcluded([...excluded, program]);
  } catch (error) {
    throw { field: "program", message: reason(error) };
  }
  return `Text copied from ${program} is not kept`;
}

async function openView(itemId: string): Promise<CustomView> {
  throw new Error(`unknown view: ${itemId}`);
}

export const command: Command = { getView, runAction, submitForm, openView };
