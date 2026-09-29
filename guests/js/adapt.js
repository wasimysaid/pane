// SPDX-License-Identifier: Apache-2.0 OR MIT
//
// Pane's adapter between a JS/TS command's exports and its component. The
// build (tools/componentize-js/pane_js.py) bundles it into every JS/TS
// component, around the objects the command exports, so that whatever a
// handler throws is an error it answers with, never a crash:
//
// - from `submitForm`, a `FormError`-like object (`{ field?, message }`) as
//   it is, and anything else (an `Error`, a string) as a message about the
//   whole form;
// - from every other handler that answers with an error (`getView`,
//   `runAction`, `openView`, a custom view's `handleEvent`, `resultsFor`,
//   `results`, `runOperation`, `runQuery`, `search`, `runTask`), the message of an `Error` or of an object
//   with a `message`, or the text of anything else.
//
// A crash is then only what a crash should be: resolving with a value of
// the wrong type, or a custom view's `render` throwing (it has no error to
// answer with).

/** The text of what a handler threw. */
function message(thrown) {
  if (thrown instanceof Error) return thrown.message;
  if (typeof thrown === "string") return thrown;
  if (thrown !== null && typeof thrown === "object" && typeof thrown.message === "string") {
    return thrown.message;
  }
  return String(thrown);
}

/** The form error for what `submitForm` threw. */
function formError(thrown) {
  const plain =
    thrown !== null &&
    typeof thrown === "object" &&
    !(thrown instanceof Error) &&
    typeof thrown.message === "string";
  if (plain && typeof thrown.field === "string") {
    return { field: thrown.field, message: thrown.message };
  }
  return { message: message(thrown) };
}

/**
 * `target[name]`, called on `target`, throwing what `error` makes of what it
 * throws, and passing what it resolves with through `then`.
 */
function adapted(target, name, error, then = (value) => value) {
  const handler = target[name];
  if (typeof handler !== "function") return handler;
  return async (...args) => {
    let value;
    try {
      value = await handler.apply(target, args);
    } catch (thrown) {
      throw error(thrown);
    }
    return then(value);
  };
}

/** `view`, a custom view, with `handleEvent` answering errors as text. */
function adaptView(view) {
  if (view !== null && typeof view === "object" && typeof view.handleEvent === "function") {
    const handleEvent = adapted(view, "handleEvent", message);
    try {
      Object.defineProperty(view, "handleEvent", { value: handleEvent, configurable: true });
    } catch {
      // A frozen view keeps its own handler.
    }
  }
  return view;
}

/** The exported `command`, adapted. */
export function adaptCommand(command) {
  if (command === null || typeof command !== "object") return command;
  return {
    ...command,
    getView: adapted(command, "getView", message),
    runAction: adapted(command, "runAction", message),
    submitForm: adapted(command, "submitForm", formError),
    openView: adapted(command, "openView", message, adaptView),
  };
}

/** An exported provider whose handler `name` answers errors as text. */
export function adaptProvider(provider, name) {
  if (provider === null || typeof provider !== "object") return provider;
  return { ...provider, [name]: adapted(provider, name, message) };
}
