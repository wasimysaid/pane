// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Declarations for `pane:extension/service-status` in wit/background.wit:
// what a running continuing service shows the user. A command imports it
// only if its bundle uses it (the build adds it to the command's world), so
// commands that do not are unchanged.

/** `pane:extension/service-status@0.1.0`. */
declare module "pane:extension/service-status@0.1.0" {
  /**
   * Shows `text` as the running service's status in Manage extensions, in
   * place of the one before. Throws with the reason anywhere but in the
   * service's own run (a command's call, a scheduled task) or once Pane
   * stopped the service. The status goes when the service ends.
   */
  export function setStatus(text: string): void;
}
