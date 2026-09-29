// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Declarations for `pane:extension/clipboard-history` in wit/clipboard.wit:
// the text the user copies, which Pane's host keeps on this computer for the
// calling package once the user turned it on (a WASI guest cannot watch the
// system's clipboard, and the history is kept while the command is not
// running). Only a command whose package.json sets
// `"pane": { "clipboardHistory": true }` imports it. Pane keeps plain text
// only, never what the copying program marked as not to be kept, nor what
// was copied from a program the user excluded; nothing is sent anywhere.
// Every function throws, on failure, an object whose `payload` is the reason.

/** `pane:extension/clipboard-history@0.1.0`. */
declare module "pane:extension/clipboard-history@0.1.0" {
  /**
   * Whether Pane keeps what is copied for this package: `off` (every
   * package starts so), `on`, or `paused` (turned on, then paused).
   */
  export type Capture = "off" | "on" | "paused";

  export interface HistoryStatus {
    capture: Capture;
    /**
     * Why Pane does not watch the clipboard although it should, or cannot on
     * this system; `undefined` or `null` while all is well.
     */
    problem?: string | null;
    /** The programs whose copied text is not kept, as lowercase file names such as `keepass.exe`. */
    excluded: string[];
    /** How many items are kept. */
    items: number;
  }

  /** One kept item. */
  export interface Entry {
    /** Identifies it to `copy`. */
    id: string;
    text: string;
    /** When it was copied, in milliseconds since the Unix epoch. */
    copiedAt: number;
    /** How long ago it was copied, in seconds. */
    ageSeconds: number;
    /** The file name of the program it was copied from, such as `notepad.exe`, if the system said. */
    source?: string | null;
  }

  export function status(): HistoryStatus;
  /**
   * Turns keeping on or off, or pauses it. Turning it on throws, and changes
   * nothing, where Pane cannot watch the clipboard.
   */
  export function setCapture(capture: Capture): void;
  /**
   * Replaces the excluded programs: file names such as `KeePass.exe`, matched
   * ignoring case, with or without their extension.
   */
  export function setExcluded(programs: string[]): void;
  /** The kept items, newest first. */
  export function entries(): Entry[];
  /** Puts the kept item `id` on the clipboard again. */
  export function copy(id: string): void;
  /**
   * Deletes every kept item; whether history is kept and the excluded
   * programs stay as they are. Returns how many were deleted.
   */
  export function clear(): number;
}
