# Extension guests

Extensions are WebAssembly components implementing the `pane:extension`
contract in [`wit/extension.wit`](../wit/extension.wit). Pane registers only
WASI 0.3 interfaces; a component that imports WASI 0.2 (for example through
Rust's standard library on `wasm32-wasip2`) is rejected with an explanation.

- `pane-guest`: Rust bindings for the contract. `no_std`, so only WASI 0.3 is
  imported; it supplies the allocator, a trapping panic handler,
  `cabi_realloc` and `memcmp`/`bcmp` (which string comparisons need).
- `sample-rust`, `sample-js`, `sample-ts`: the same sample command in Rust,
  JavaScript and TypeScript. All three show the same items, the same form and
  the same color picker, compute the same root result ("reverse <text>"), and
  give the same answers and errors; the contract
  tests in `crates/pane-core/tests/samples.rs` and `crates/pane/tests/window.rs`
  hold each of them to that.
- `sample-settings`, `sample-settings-js`, `sample-settings-ts`: the same
  command in Rust, JavaScript and TypeScript, which keeps a chosen greeting
  style in Pane's settings ([Keeping settings](#keeping-settings)) and one
  value of each other kind of data
  ([content, cache and credentials](#keeping-content-cache-and-credentials));
  the fixtures for disabling and re-enabling a package and for clearing its
  cache, held alike by `crates/pane-core/tests/disable.rs` and
  `crates/pane-core/tests/clear_cache.rs`. Their **Crash** item crashes on
  purpose: three crashes within five minutes pause the package until Retry
  ([pausing](../docs/pausing.md), `crates/pane-core/tests/pausing.rs`).
- `calculator`: Pane's calculator, a default extension in Rust: an
  arithmetic expression typed into root search lists its answer, which Enter
  copies ([Root results](#root-results-computed-from-the-query),
  [expression scope](../docs/root-search.md#the-calculator)). Its package
  is `packages/calculator`; held by `crates/pane-core/tests/calculator.rs`.
- `applications`: Pane's application launcher, a default extension in
  Rust: the installed applications, which Pane's host finds, are found by
  name in root search and Enter opens one
  ([Root results supplied ahead of the query](#root-results-supplied-ahead-of-the-query),
  [applications](../docs/applications.md)). Its package is
  `packages/applications`; held by `crates/pane-core/tests/applications.rs`.
- `files`: Pane's file search, a default extension in Rust: the user grants
  it a folder through Pane's own row, and root search finds its files by
  name and opens one ([Files of a granted folder](#files-of-a-granted-folder),
  [files](../docs/files.md)).
  Its package is `packages/files`; held by `crates/pane-core/tests/files.rs`.
- `sample-files-js`, `sample-files-ts`: the same host import and `open-file`
  results in JavaScript and TypeScript; held by
  `crates/pane-core/tests/files.rs`.
- `sample-helper`, `sample-helper-js`, `sample-helper-ts`: a command in
  Rust, JavaScript and TypeScript running a [native helper](#native-helpers)
  its package ships, `helpers/echo` (`pane-echo`, an ordinary program
  `cargo xtask guests` builds for the system it runs on and puts in all
  three packages): its answer, cancelling it, a failing and an undeclared
  helper, and a slow run that disabling or reloading stops. Their packages
  are `packages/sample-helper`, `packages/sample-helper-js` and
  `packages/sample-helper-ts`; held alike by
  `crates/pane-core/tests/helpers.rs`.
- `sample-applications-js`, `sample-applications-ts`: the same host import
  and indexed results in JavaScript and TypeScript: "Launch <name>" for each
  installed application, and a command listing and opening them
  ([Root results supplied ahead of the query](#root-results-supplied-ahead-of-the-query));
  held by `crates/pane-core/tests/applications.rs`.
- `sample-operations`, `sample-operations-js`, `sample-operations-ts`: each
  package publishes the operation `greet` and has a command that calls
  another's, Rust calling JavaScript and TypeScript and they calling Rust
  ([Operations](#operations)); held by
  `crates/pane-core/tests/operations.rs`.
- `sample-dependencies`: a Rust package declaring the JavaScript operations
  sample as a required dependency and the Rust one as an optional one, and
  calling each by its dependency id; installing it installs the JavaScript
  sample too ([Dependencies](#dependencies-on-other-extensions)); held by
  `crates/pane-core/tests/dependencies.rs`.
- `npm/greeter`: `@pane-samples/greeter`, the npm-distributed sample: an
  npm package holding a `pane.json` and one JavaScript component,
  `sample_npm_js.wasm` (from `sample-npm-js`, prebuilt like the other
  JavaScript samples), whose command answers "Hello from the npm package"
  and whose `greet` operation answers "Hello, <name>, from the npm package",
  so it is plain which copy runs. `"private": true` keeps `npm publish` from
  publishing it;
  `cargo xtask guests` assembles it in `target/guests/npm/greeter/` and packs
  it as `npm pack` does into `target/guests/npm/pane-samples-greeter-0.1.0.tgz`
  ([Publishing a package to npm](#publishing-a-package-to-npm)).
  `packages/sample-dependencies-npm` is the dependencies sample's component
  requiring it as `npm:@pane-samples/greeter`; held by
  `crates/pane-core/tests/npm.rs` and `crates/pane/tests/npm.rs`, from a
  local registry.
- `sample-query`, `sample-query-js`, `sample-query-ts`: Echo, the smallest
  command that takes a query, in Rust, JavaScript and TypeScript:
  it answers the text the user sends it from root search through its alias
  or as a fallback ([A command that takes a query](#a-command-that-takes-a-query),
  [aliases and fallbacks](../docs/aliases.md)); "fail" is refused and
  "crash" crashes on purpose. Their packages are `packages/sample-query`,
  `packages/sample-query-js` and `packages/sample-query-ts`; held alike by
  `crates/pane-core/tests/aliases.rs`, and the Rust one by
  `crates/pane/tests/aliases.rs`.
- `hello-rust`, `hello-js`, `hello-ts`: one "Say hello" command each, a
  package built in its own folder, as an author's would be, for
  [development mode](../docs/development-mode.md): Pane builds and reloads
  it after each save ([Developing a package](#developing-a-package-build-and-reload-on-save));
  held by `crates/pane-core/tests/develop_builds.rs`.
- `js`: `@pane/extension`, TypeScript declarations for the contract
  (`pane.d.ts`) and the WIT world JS/TS commands are built against.
- `prebuilt`: the JS and TS sample components (both samples in each
  language), committed so that tests and
  `cargo run -p pane` need no JavaScript toolchain, with `manifest.json`
  recording their hashes and build inputs.
- `packages`: the samples', the calculator's and applications' package manifests (`pane.json`). `cargo xtask
  guests` puts each one with its built component in
  `target/guests/packages/<name>/`, a ready-to-install package.
- `fixtures/faulty`: test fixture whose actions, form, custom view and root
  results return an error or trap.
- `fixtures/failing-start`: test fixture that builds and installs but traps
  the first time it is asked for its view (after saving a setting), so a
  reload to it fails to start and Retry then starts it.
- `fixtures/refusing-view`: test fixture whose view is always refused with
  an error it returns, which a reload must not report as a failure to start.
- `fixtures/operations`: test fixture installed as several packages to drive
  each way an operation call can fail, cycles and the depth limit.
- `fixtures/mixed-p2`: negative control that imports WASI 0.2 and must be rejected.
- `fixtures/old-api`: negative control built against extension API 0.1 as it
  was before `item` gained `platforms` and custom views, with its own copy of
  that WIT; Pane's type check refuses it at install and when it loads.
- `fixtures/mismatched-api`: negative control whose exports all have the
  names Pane looks for while `item` lacks one field, so only the type check
  can refuse it.

## Writing a Rust command

The [sample](sample-rust/src/lib.rs) is the complete example. A command is a
`cdylib` crate depending on `pane-guest` that implements four async
functions and names its custom view type (see [Forms](#forms) and
[Custom views](#custom-views) for the last two):

```rust
#![no_std]

use pane_guest::alloc::{string::String, vec, vec::Vec};
use pane_guest::{CustomView, FieldValue, FormError, Guest, Item, NoCustomView, View};

struct Hello;
pane_guest::export!(Hello);

impl Guest for Hello {
    type CustomView = NoCustomView;

    async fn get_view() -> Result<View, String> {
        let item = Item {
            id: "hi".into(),
            title: "Say hi".into(),
            subtitle: None,
            form: None,
            platforms: None,
            custom_view: None,
        };
        Ok(View { title: "Hello".into(), items: vec![item] })
    }

    async fn run_action(_item_id: String) -> Result<String, String> {
        Ok("hi!".into())
    }

    async fn submit_form(_item_id: String, _values: Vec<FieldValue>) -> Result<String, FormError> {
        Err(FormError { field: None, message: "this command has no forms".into() })
    }

    async fn open_view(_item_id: String) -> Result<CustomView, String> {
        Err("this command has no custom views".into())
    }
}
```

Returning `Err` shows the message as an error; a panic traps the guest, which
Pane reports and recovers from by starting a fresh instance on the next call.
WASI 0.3 interfaces are available through the
[`wasip3`](https://docs.rs/wasip3/0.9.0/wasip3/) crate with
`default-features = false`; the sample awaits `wasi:clocks` this way.

Build with the pinned toolchain (`wasm32-wasip2` is the compiler target name;
the emitted component imports only WASI 0.3):

```sh
cargo xtask guests
# or, for the guests workspace only:
cd guests && cargo build --release --target wasm32-wasip2
```

To try a rebuilt sample in the launcher, `cargo run -p pane` from the root; set
`PANE_EXTENSIONS_DIR` to a directory containing `sample_rust.wasm`,
`sample_js.wasm` and `sample_ts.wasm` to use different builds. To run your
own command, make it a package and install it; see
[Packaging and installing a local extension](#packaging-and-installing-a-local-extension).

Toolchain used: Rust 1.98.1, `wit-bindgen` 0.62.0, `wasip3` 0.9.0+wasi-0.3.0;
host Wasmtime and wasmtime-wasi 49.0.1.

### Keeping settings

A command of an installed package can keep string values between runs with
the `pane:extension/settings` interface in
[`wit/data.wit`](../wit/data.wit). The settings sample, in
[Rust](sample-settings/src/lib.rs), [JavaScript](sample-settings-js/src/index.js)
and [TypeScript](sample-settings-ts/src/index.ts), saves the greeting style
the user picks. In Rust it is `pane_guest::settings`:

```rust
use pane_guest::settings;

settings::set("greeting-style", "formal")?;          // Result<(), String>
let style: Option<String> = settings::get("greeting-style")?;
```

In JavaScript and TypeScript it is a module (typed in
[`js/data.d.ts`](js/data.d.ts)); an error is thrown as an `Error`
whose message is the reason, so rethrowing it shows the reason to the user:

```ts
import { get, set } from "pane:extension/settings@0.1.0";

set("greeting-style", "formal");
const style: string | null = get("greeting-style");
```

- Values belong to the installed package's source identity, not its title
  or managed copy: two installed copies of the same package have separate
  settings, and an update keeps them.
- They are kept while the package is disabled and while Pane is not running,
  and the command sees them again when the package is enabled. While it is
  disabled nothing of the package runs, and once it is disabled a call still
  finishing from before cannot save: `set` fails instead of writing.
- Pane keeps them in `extensions/settings.json` in its data folder. If that
  file cannot be read, `get` and `set` return the reason and Pane does not
  overwrite the file. Each `set` replaces the file whole (a crash leaves the
  old or the new file), but it is not locked: two Pane processes using the
  same data folder can lose each other's last write. Keeping to one running
  Pane is a later concern.
- A command built into Pane rather than installed from a package has no
  settings: `get` and `set` return an error.
- A component that does not import `settings` is unaffected; it is built for
  the `extension` world as before. `extension-with-data` adds the imports
  within extension API 0.1, so a component that uses settings needs a Pane
  with this change. JavaScript and TypeScript commands are built against a
  world that includes it, so the prebuilt JS/TS components list the import
  whether or not they use it.

### Keeping content, cache and credentials

Next to `settings`, [`wit/data.wit`](../wit/data.wit) has three
interfaces with the same `get` and `set`, one per other kind of
[extension data](../docs/extension-data.md): `content` for the extension's own
durable records, `cache` for values it can make again, and `credentials` for
secrets kept on this computer. The settings sample uses all three. In Rust
they are `pane_guest::{content, cache, credentials}`; in JavaScript and
TypeScript the modules `pane:extension/content@0.1.0`,
`pane:extension/cache@0.1.0` and `pane:extension/credentials@0.1.0`:

```ts
import * as cache from "pane:extension/cache@0.1.0";

const greeting = cache.get("last-greeting") ?? makeGreeting();
cache.set("last-greeting", greeting);
```

- They behave like settings: owned by the source identity, kept while the
  package is disabled or updated, refused while it is disabled, and each kept
  in its own file (`content.json`, `cache.json`, `credentials.json`).
- The user can clear an extension's cache in Manage extensions at any time,
  without the extension running: expect any cache value to be missing. Its
  settings, content and credentials are kept.
- Uninstalling removes the cache and credentials; the user chooses whether
  the settings and content are kept for a later install of the same source.
  An extension installed again may therefore find settings and content
  without a cache or a credential.
- Credentials are plain text in Pane's data folder, not in the system's
  keychain; on macOS and Linux only the user can read their file. Other
  extensions and programs running as the user can
  ([limits](../docs/extension-data.md#limits)).
- The extension migrates its own values between its versions; Pane keeps
  them unchanged across an update.

## Writing a JavaScript or TypeScript command

The [JavaScript](sample-js/src/index.js) and
[TypeScript](sample-ts/src/index.ts) samples are complete examples. A command
is an npm package whose `main` module exports `command` with four async
functions (see [Forms](#forms) and [Custom views](#custom-views) for the last
two). Pane's types come from
`@pane/extension` (a `file:../js` development dependency); they describe plain
values, not engine objects:

```ts
import type { Command } from "@pane/extension";
import { waitFor } from "wasi:clocks/monotonic-clock@0.3.0";

export const command: Command = {
  async getView() {
    return { title: "Hello", items: [{ id: "hi", title: "Say hi" }] };
  },
  async runAction(itemId) {
    if (itemId !== "hi") throw new Error(`unknown item: ${itemId}`);
    await waitFor(10_000_000); // 10 ms; the command suspends meanwhile
    return "hi!";
  },
  async submitForm() {
    throw { message: "this command has no forms" };
  },
  async openView() {
    throw new Error("this command has no custom views");
  },
};
```

Throwing (a rejected promise) shows the error's message, or a thrown string,
as an error. Returning a value of the wrong type, such as `undefined` from
`runAction`, traps the guest, which Pane reports and recovers from as for
Rust. npm dependencies are bundled into the component; the samples use
[Zod](https://zod.dev) 4.6.5 (`zod/mini`) and show its validation failure as a
normal error. Only ECMAScript built-ins are available, not Node.js or browser
APIs; WASI 0.3 imports declared by [the world](js/wit/world.wit) (currently
`wasi:clocks/monotonic-clock`, and `wasi:http/client` for a command whose
bundle imports it) are imported by name;
the clock is typed in [`js/wasi.d.ts`](js/wasi.d.ts), and web requests have
a typed helper, [`@pane/extension/http`](#searching-inside-a-command). `Math.random`, `Date.now()` and
`performance.now()` are fresh in each instance.

**Snapshot caveat.** A component is built by running the module once and
snapshotting the engine, so module top-level code runs at build time, on the
build machine, and every instance starts from its result. Keep top-level code
to pure setup such as schemas and constants: secrets, IDs, timestamps, random
values or anything else meant to differ per instance belong inside
`getView`/`runAction`. For the same reason, rebuilt components are never
byte-identical.

Build commands, from the repository root:

```sh
cargo xtask js-guests        # rebuild guests/prebuilt/ and target/guests/ from the samples
python3 tools/componentize-js/pane_js.py build <package dir> <out.wasm>   # any command package
python3 tools/componentize-js/pane_js.py check   # are the prebuilt samples current and their npm licenses permissive?
```

A build installs the package's locked dependencies into a staging copy,
type-checks it with TypeScript when it has a `tsconfig.json` (the JS sample is
checked through JSDoc), bundles it with esbuild and componentizes it. The TS
sample's `.ts` source is transpiled by esbuild and runs on the same runtime as
JS. The first run builds the toolchain (about five minutes), later runs take
seconds. Everything downloaded or built goes to `PANE_JS_TOOLCHAIN_DIR`,
default `~/.cache/pane/componentize-js` on Linux,
`~/Library/Caches/pane/componentize-js` on macOS and
`%LOCALAPPDATA%\pane\componentize-js` on Windows; the source tree is not
written except for the output. After editing a sample, run
`cargo xtask js-guests` and commit the updated `guests/prebuilt/`.

Prerequisites, in addition to the Rust ones in the [README](../README.md):

- Python 3.12 or later (`python3`; on Windows use `python` in the commands
  above; set `PYTHON` for `cargo xtask` if yours is named differently), git,
  and Node.js 22 or later with npm. `cargo xtask ci` also runs the `check`
  subcommand, so it needs Python too.
- The build installs Rust `nightly-2026-09-27` with `rust-src` through rustup,
  and downloads wasi-sdk 34 for the host (x86_64 or arm64, all three OSes).
- **Windows:** `python` from python.org or the Microsoft Store and Node.js
  from nodejs.org; run from a normal shell. **macOS:** Xcode Command Line
  Tools already provide git; install Python and Node.js from their installers
  or Homebrew. **Linux:** the distribution's `python3`, `git`, `nodejs` and
  `npm` (Node.js 22+, for example through nvm).

Toolchain used: upstream [componentize-qjs](https://github.com/andreiltd/componentize-qjs)
0.4.5 at `e563c6d6` with the three patches in
[`tools/componentize-js/patches`](../tools/componentize-js/patches), its
QuickJS runtime built with `nightly-2026-09-27` for `wasm32-wasip3` against
wasi-sdk 34, the componentizer built with Rust 1.98.1, esbuild 0.28.2 and
TypeScript 7.0.2. Every JS component imports the same 20 WASI 0.3 interfaces
through its libc, whatever the source uses, and `wasi:http`'s `types` and
`client` too if its bundle imports `wasi:http` (itself or through
`@pane/extension/http`), which Pane then lists as using the network; it is
about 4.4 MB. Only Linux x86_64 builds have been run; the scripts avoid OS-specific paths, but Windows
and macOS builds are unverified. See
[tools/componentize-js](../tools/componentize-js/README.md) for the patch queue.

## Forms

An item can open a form instead of running an action: a single-line text
field and a choice of one option per field, and a submit button. Pane renders
the controls, handles focus, typing and input methods, and calls
`submit-form` with every field's value; the command validates them and answers
with a result, or with an error about one field (shown under it, with focus
moved there) or about the whole form. The contract, keyboard behavior and
accessibility are described in [docs/forms.md](../docs/forms.md). The "Greet
someone" item of each sample is the complete example.

Rust:

```rust
use pane_guest::{Choice, Field, FieldKind, FieldValue, Form, FormError, Item, TextField};

let form = Form {
    title: "Greet someone".into(),
    fields: vec![
        Field {
            id: "name".into(),
            label: "Name".into(),
            kind: FieldKind::Text(TextField { placeholder: Some("Ada Lovelace".into()) }),
        },
        Field {
            id: "greeting".into(),
            label: "Greeting".into(),
            kind: FieldKind::Choice(vec![
                Choice { id: "hello".into(), label: "Hello".into() },
                Choice { id: "morning".into(), label: "Good morning".into() },
            ]),
        },
    ],
    submit_label: "Greet".into(),
};
let item = Item { id: "form".into(), title: "Greet someone".into(), subtitle: None, form: Some(form), platforms: None };

// In `impl Guest`:
async fn submit_form(item_id: String, values: Vec<FieldValue>) -> Result<String, FormError> {
    let name = values.iter().find(|v| v.id == "name").map_or("", |v| v.value.trim());
    if name.is_empty() {
        return Err(FormError { field: Some("name".into()), message: "Enter a name".into() });
    }
    Ok(format!("Hello, {name}"))
}
```

JavaScript or TypeScript (fields are camelCase; a field's `kind` is a tagged
value, `{ tag: "text", val: {...} }` or `{ tag: "choice", val: [...] }`):

```ts
const form: Form = {
  title: "Greet someone",
  fields: [
    { id: "name", label: "Name", kind: { tag: "text", val: { placeholder: "Ada Lovelace" } } },
    { id: "greeting", label: "Greeting", kind: { tag: "choice", val: [
      { id: "hello", label: "Hello" }, { id: "morning", label: "Good morning" },
    ] } },
  ],
  submitLabel: "Greet",
};
// items: [{ id: "form", title: "Greet someone", form }]

async submitForm(itemId, values) {
  const name = values.find((v) => v.id === "name")?.value.trim() ?? "";
  if (!name) throw { field: "name", message: "Enter a name" } satisfies FormError;
  return `Hello, ${name}`;
},
```

In JS/TS, reject a submission by throwing a plain `FormError` object as above.
Throwing an `Error` (or a string) from `submitForm` rejects the form as a
whole with its message. The samples validate with Zod and turn its first
issue into a `FormError`.

## Errors and crashes

An error a command returns (Rust `Err`; in JS/TS, anything a handler
throws) is its message to the user: Pane shows it and the command keeps
running, however often it happens. The JS/TS build wraps the exported
handlers ([`guests/js/adapt.js`](js/adapt.js)) so that a thrown `Error`,
string or `{ message }` object is always such an error. A crash is
different: a Rust panic, or in JS/TS resolving with a value of the wrong
type (or a custom view's `render` throwing), traps the guest. Pane reports it and starts a fresh instance for the next
call; after three crashes within five minutes, or a component that cannot
start, Pane pauses the whole package until the user chooses Retry in
**Manage extensions…** (where "Why <title> is paused" shows the details),
keeping its data ([pausing](../docs/pausing.md)).
So report expected failures, such as a missing sign-in, as errors, never by
crashing. The settings samples' **Crash** item shows a crash in each
language.

Pane's extension runtime itself can crash too (a fault in Pane, not in any
extension). Pane then stops every call in progress and never runs one again
by itself, so an action that did its work (saving, sending a request) may
have lost only its answer: the user sees that the runtime stopped and runs
it again only if they want it done again
([runtime crashes](../docs/pausing.md#when-the-extension-runtime-itself-crashes)).
Write an action whose repetition matters so the user can tell whether it
ran, as the Rust settings sample's **Count** does by answering the count it
saved.

## Actions for some operating systems only

An item can list the operating systems its action (or form) works on. On any
other system Pane still lists it, shows why it is unavailable ("Not available
on Linux: this action supports only Windows") and never calls the command for
it, so the rest of the command keeps working:

```rust
Item {
    platforms: Some(vec![Platform::Windows]), // pane_guest::Platform
    ..item("windows-only", "Windows-only action", "Declared to work on Windows only")
}
```

```js
{ id: "windows-only", title: "Windows-only action", platforms: ["windows"] }
```

`platforms` is `None` / omitted for every system. The command cannot tell
which system it runs on; Pane applies the declaration. The samples' last two
items are the runnable example, and
[platform availability](../docs/platform-availability.md) has the details.

## Root results computed from the query

A command can answer what the user types into root search, as the
[calculator](calculator) does: its results are listed above the results
root search finds by title, and Enter on one performs its action:
copying a text to the clipboard (`copy`) or opening an `http://` or
`https://` address with the system's handler for web links, normally the
default browser (`open-url`, as [quicklinks](quicklinks) do; Pane refuses
any other address). Set `"rootResults": true` on the
command in `pane.json` and export `pane:extension/root-results`
([`wit/root-results.wit`](../wit/root-results.wit)) beside the command.
Pane asks the command on every change of a query that is not blank, so its
instance starts with the first query typed, and discards an answer once the
query has changed. A query the command has no answer for returns no results
(an incomplete expression is not an error); returning an error is the
extension failing, and Pane lists a result explaining it. A disabled package
is not asked. See [root search](../docs/root-search.md#results-computed-from-the-query).

Rust (`pane_guest::root`; the component then exports both interfaces):

```rust
use pane_guest::alloc::{string::String, vec, vec::Vec};
use pane_guest::root::{RootAction, RootResult};

pane_guest::export!(Sample);
pane_guest::root::export!(Sample);

impl pane_guest::root::Guest for Sample {
    async fn results_for(query: String) -> Result<Vec<RootResult>, String> {
        let Some(text) = query.strip_prefix("reverse ") else {
            return Ok(Vec::new());
        };
        let reversed: String = text.chars().rev().collect();
        Ok(vec![RootResult {
            id: "reversed".into(),
            title: reversed.clone(),
            subtitle: None,
            action: RootAction::Copy(reversed),
        }])
    }
}
```

JavaScript or TypeScript: add `"pane": { "rootResults": true }` to
`package.json`, so the build exports the interface, and export
`rootResults` from the module:

```ts
import type { RootResult, RootResults } from "@pane/extension";

export const rootResults: RootResults = {
  async resultsFor(query): Promise<RootResult[]> {
    if (!query.startsWith("reverse ")) return [];
    const reversed = [...query.slice(8)].reverse().join("");
    return [{ id: "reversed", title: reversed, action: { tag: "copy", val: reversed } }];
  },
};
```

The three samples answer "reverse <text>" this way, and "pane website"
with a result whose action opens a link (`RootAction::OpenUrl(url)` in Rust,
`{ tag: "open-url", val: url }` in JavaScript and TypeScript); their
packages in [`packages/`](packages) set `rootResults`.

Once the query changes or root search is left, Pane cancels a call still
pending: one not started is never started, and one waiting inside the
command (on an async import) is dropped with the command's instance, so
module or struct state kept between queries is lost and the next query
starts a fresh instance. Keep what must last in [settings](#keeping-settings)
or the [cache](#keeping-content-cache-and-credentials).

### Files of a granted folder

A command can find the files of the one folder the user granted its
package, which a WASI guest cannot read itself, through
`pane:extension/files` ([`wit/files.wit`](../wit/files.wit)), and answer
results that open one (`open-file`). The package's `pane.json` sets
`"folderAccess": true`: Pane then shows its own "Choose folder…" row at the
top of the package's commands, and records the folder the user picks. The
command never names or sees a path: `list-folder()` answers that no folder
is granted, that Pane is listing it (Pane asks the command again once it is
done, so answer no files for now), or the listing Pane keeps for this visit
of root search, whose files have an `id` and a `relative` path. An
`open-file` result gives the `id`; Pane shows the file's own name and folder
in the row, whatever the result's title says, drops an id it did not give,
checks the file again when it is invoked and refuses programs and scripts.
Pane lists the folder under its [scan policy](../docs/files.md#the-scan-policy)
(`files.limits()` gives its limits); file results are listed after the
results root search finds by title. The [Files](files) default extension,
in Rust, works this way; [`sample-files-js`](sample-files-js) and
[`sample-files-ts`](sample-files-ts) do the same in JavaScript and
TypeScript.

Rust (`pane_guest::files`):

```rust
use pane_guest::files::{self, FolderState};
use pane_guest::root::{RootAction, RootResult};

async fn results_for(query: String) -> Result<Vec<RootResult>, String> {
    let FolderState::Ready(listing) = files::list_folder()? else {
        return Ok(Vec::new());
    };
    Ok(listing
        .files
        .into_iter()
        .filter(|file| file.relative.contains(query.as_str()))
        .map(|file| RootResult {
            id: file.relative.clone(),
            title: file.relative,
            subtitle: None,
            action: RootAction::OpenFile(file.id),
        })
        .collect())
}
```

JavaScript or TypeScript: add `"files": true` to the `"pane"` options of
`package.json`, so the build imports the interface (a command without it
does not), and import it (`listFolder` throws an object whose `payload` is
the reason; declarations in [`js/files.d.ts`](js/files.d.ts)):

```ts
import { listFolder } from "pane:extension/files@0.1.0";

const state = listFolder();
if (state.tag !== "ready") return [];
return state.val.files
  .filter((file) => file.relative.includes(query))
  .map((file) => ({ id: file.relative, title: file.relative, action: { tag: "open-file", val: file.id } }));
```

## Root results supplied ahead of the query

A command can also give root search results that do not depend on the
query, as the [applications](applications) extension gives the installed
applications: Pane asks once root search is used, keeps them, and matches
and ranks them by title like commands, for a query that is not blank. Set
`"indexedResults": true` on the command in `pane.json` and export
`pane:extension/indexed-results` ([`wit/applications.wit`](../wit/applications.wit))
beside the command. Pane asks again after each return to root search; an
error is listed as a row explaining it. The only action is opening an
installed application. See [root search](../docs/root-search.md#results-supplied-ahead-of-the-query)
and [applications](../docs/applications.md).

Any Rust command can also find and open the installed applications through
Pane (`pane_guest::applications`, the `pane:extension/applications`
import), since a WASI guest cannot:

```rust
use pane_guest::alloc::{string::String, vec::Vec};
use pane_guest::applications;
use pane_guest::indexed::{IndexedAction, IndexedResult};

pane_guest::export!(Apps);
pane_guest::indexed::export!(Apps);

impl pane_guest::indexed::Guest for Apps {
    async fn results() -> Result<Vec<IndexedResult>, String> {
        Ok(applications::installed()?
            .into_iter()
            .map(|app| IndexedResult {
                action: IndexedAction::OpenApplication(app.id.clone()),
                id: app.id,
                title: app.name,
                subtitle: Some("Application".into()),
            })
            .collect())
    }
}
```

`applications::open(&id)` opens one from a command's own action.

JavaScript or TypeScript: add `"pane": { "indexedResults": true }` to
`package.json`, so the build exports the interface, import the host's
functions from `"pane:extension/applications@0.1.0"` (they throw an object
whose `payload` is the reason) and export `indexedResults`:

```ts
import type { IndexedResults } from "@pane/extension";
import { installed } from "pane:extension/applications@0.1.0";

export const indexedResults: IndexedResults = {
  async results() {
    return installed().map((app) => ({
      id: app.id,
      title: `Launch ${app.name}`,
      action: { tag: "open-application", val: app.id },
    }));
  },
};
```

The [JavaScript](sample-applications-js) and
[TypeScript](sample-applications-ts) applications samples do this, and
their commands list the applications and open one with `open(id)`; their
packages in [`packages/`](packages) set `indexedResults`.

## A scheduled task

A command can run a task in the background every so many minutes, from 1
to 1440, once the user turns its schedule on in Manage extensions
(installing schedules nothing). Declare the schedule on the command in
`pane.json`, and export `pane:extension/scheduled-task`
([`wit/background.wit`](../wit/background.wit)) beside the command:

```json
{ "id": "ticks", "title": "Ticks", "component": "ticks.wasm",
  "schedule": { "everyMinutes": 15 } }
```

```rust
pane_guest::export!(Ticks);
pane_guest::scheduled::export!(Ticks);

impl pane_guest::scheduled::Guest for Ticks {
    async fn run_task(command: String) -> Result<String, String> {
        // One run: its answer is shown as the task's latest result, an
        // error as its latest failure; a panic is a crash of the package.
        Ok("Done".into())
    }
}
```

JavaScript or TypeScript: add `"pane": { "scheduledTask": true }` to
`package.json` and export `scheduledTask`:

```ts
import type { ScheduledTask } from "@pane/extension";

export const scheduledTask: ScheduledTask = {
  async runTask(command) {
    return "Done";
  },
};
```

What to expect ([scheduled tasks](../docs/background.md)):

- Turned on, the task runs at once, then one interval after each run
  began, never two runs at once; what fell due while Pane was closed or the
  package disabled or paused runs once when it can.
- Each run starts in an **instance of its own**: keep what the next run
  needs in [extension data](#keeping-content-cache-and-credentials), not in
  memory. Settings and content work as in a call; calling other packages'
  operations is refused.
- Disabling, reloading, updating or uninstalling the package, or turning
  the schedule off, **stops a run where it awaits**: nothing after the
  `await` runs, and its answer is discarded. Save what must survive a stop
  before awaiting.
- **Await rather than compute for long**: every extension's calls are
  served on one thread, which a run computing without yielding holds.
- An error is shown and the schedule goes on; crashes count towards
  [pausing](../docs/pausing.md) the package like any crash.

The background samples ([Rust](sample-background),
[JavaScript](sample-background-js), [TypeScript](sample-background-ts);
packages in [`packages/`](packages)) count each run, and their command
chooses how the next runs behave (answer, fail, crash or wait 10 seconds).

## A command that takes a query

The user can give any installed command an alias in Manage extensions, and
typing it in root search lists the command first; nothing is needed of the
command for that. A command that **takes a query** can also be sent text
from root search: the user types its alias, a space and the text ("ec
hello"), or makes it a fallback, which is listed below the results for any
text typed, and invokes that row. Pane calls the command only then, never
while the user types, and shows its answer as the result (an error as the
failure); root search stays as it was. Set `"takesQuery": true` on the
command in `pane.json` and export `pane:extension/query-command`
([`wit/query.wit`](../wit/query.wit)) beside the command; Pane checks it at
install without running it. Pane passes the command's id in `pane.json`, so
one component can serve several such commands, and the text, trimmed and
never empty. A trap counts towards [pausing](../docs/pausing.md) as any
call's does. See [aliases and fallbacks](../docs/aliases.md).

Rust (`pane_guest::query`; the component then exports both interfaces), as
[`sample-query`](sample-query) does:

```rust
use pane_guest::alloc::{format, string::String};

pane_guest::export!(Echo);
pane_guest::query::export!(Echo);

impl pane_guest::query::Guest for Echo {
    async fn run_query(command: String, query: String) -> Result<String, String> {
        Ok(format!("Echo heard “{query}”"))
    }
}
```

JavaScript or TypeScript: add `"pane": { "takesQuery": true }` to
`package.json`, so the build exports the interface, and export
`queryCommand` from the module, as the [JavaScript](sample-query-js) and
[TypeScript](sample-query-ts) query samples do:

```ts
import type { QueryCommand } from "@pane/extension";

export const queryCommand: QueryCommand = {
  async runQuery(command, query) {
    return `Echo heard “${query}”`;
  },
};
```

## Searching inside a command

A command that searches an online service as the user types sets
`"search": true` on its entry in `pane.json` and exports
`pane:extension/command-search` ([wit/search.wit](../wit/search.wit)) beside
`command`. Pane gives it a search field of its own once the user opens it
and calls `search(command, query)` with the text typed there (trimmed,
never empty); the results (`id`, `title`, optional `subtitle`) replace the
command's list, and activating one calls `run-action` with its id. Root
search never calls it, so nothing typed there reaches the command or its
service. Pane waits 150 ms before it starts a search, and stops one it no
longer needs (the text changed, the user left) where it waits, dropping the
instance with its web request: code after that `await` never runs and
in-memory state is lost, so make result ids say which result they are. An
error it answers with (a service down or unreachable) is shown in place of
results and never pauses the extension. A command cannot set both
`"search"` and `"rootResults"`. See
[docs/command-search.md](../docs/command-search.md).

**Web requests** go through `wasi:http@0.3.0`'s client, which Pane links for
every command and sends from the host (`http` and `https` over HTTP/1.1,
trusting the system's certificates, no redirects followed). Pane bounds each
request, whatever its options ask: 10 s to connect, 20 s for the response
head, 10 s between two pieces of the body, 30 s in all, a body of at most
4 MiB, and four connections open at once per package; past a limit the
request fails with an error saying so. Any address is allowed, this
computer's and the local network's too; Manage extensions shows which
packages use the network and the addresses each tried to reach this
session. The SDKs wrap it:

```rust
// Rust (no_std): pane_guest::http, the generated wasi:http bindings beside it.
let response = pane_guest::http::get(&url, &[("accept", "application/json")]).await?;
if response.status != 200 { return Err(format!("the service answered {}", response.status)); }
let found: Found = serde_json::from_slice(&response.body).map_err(|e| e.to_string())?;
```

```ts
// JS/TS: bundled into the component like any npm module.
import { get } from "@pane/extension/http";
const response = await get(url, { accept: "application/json" }); // throws Error("connection refused")...
const found = response.json();
```

A failure to get a response is an error whose message says why
("connection refused", "the address could not be resolved", "the host's
certificate is not trusted", "the service did not answer in time", "the
answer is larger than the 4194304 bytes Pane accepts"); any status is a
response. For other methods, request bodies or streaming, use the
standard bindings (`pane_guest::http::wasi::http`, or
`wasi:http/types@0.3.0` and `wasi:http/client@0.3.0` in JS, untyped).
Libraries built on `wasi:http` work; ones opening sockets themselves, or
needing Node.js or browser `fetch`, do not. Rust crates must build for
`no_std` + `alloc` (the sample uses `serde` and `serde_json` that way).

The samples ([Rust](sample-search/src/lib.rs),
[JavaScript](sample-search-js/src/index.js),
[TypeScript](sample-search-ts/src/index.ts)) search the fixture service, a
made-up package registry on this computer: run
`cargo run -p pane-core --example fixture_service` (port 8740, their
default address; `-- --port N` for another, which their item "Service
address" then sets), install
`target/guests/packages/sample-search`, open Package search and type.

## Custom views

An item can open a custom view that the command draws itself: filled
rectangles and one-line text in a fixed-size area, redrawn after each key
(arrows, Home, End) or pointer event (press over the view, drag, release).
Pane keeps focus and the focus ring, and exposes the view to assistive
technology as one control with the item's label and role and the value the
view reports. The command keeps each open view's state in a `custom-view`
resource that `open-view` returns; Pane drops it when the view closes. The
contract, input, lifecycle and accessibility are described in
[docs/custom-views.md](../docs/custom-views.md). The "Choose a color" item of
each sample is the complete example.

Rust (the view is a type implementing `GuestCustomView`; its methods take
`&self`, so state goes in `Cell`s or `RefCell`s):

```rust
use core::cell::Cell;
use pane_guest::alloc::{format, string::String, vec};
use pane_guest::{
    CustomView, CustomViewInfo, CustomViewRole, Frame, GuestCustomView, Key, Rect, Shape, ViewEvent,
};

struct Picker { column: Cell<i32> }

impl GuestCustomView for Picker {
    async fn render(&self) -> Frame {
        let x = self.column.get() * 36;
        Frame {
            width: 288,
            height: 36,
            shapes: vec![Shape::Rect(Rect { x, y: 0, width: 36, height: 36, fill: 0x1e88e5 })],
            value: format!("Column {}", self.column.get() + 1),
        }
    }

    async fn handle_event(&self, event: ViewEvent) -> Result<(), String> {
        match event {
            ViewEvent::Key(Key::Right) => self.column.set((self.column.get() + 1).min(7)),
            ViewEvent::Key(Key::Left) => self.column.set((self.column.get() - 1).max(0)),
            ViewEvent::PointerDown(at) => self.column.set((at.x / 36).clamp(0, 7)),
            _ => {}
        }
        Ok(())
    }
}

// The item: `custom_view: Some(CustomViewInfo { title: "Pick".into(), label:
// "Column".into(), role: CustomViewRole::ColorWell })`. In `impl Guest`:
type CustomView = Picker;

async fn open_view(_item_id: String) -> Result<CustomView, String> {
    Ok(CustomView::new(Picker { column: Cell::new(0) }))
}
```

JavaScript or TypeScript (a view is any object with `async render()` and
`async handleEvent(event)`; shapes and events are tagged values):

```ts
class Picker implements CustomView {
  column = 0;
  async render(): Promise<Frame> {
    return {
      width: 288,
      height: 36,
      shapes: [{ tag: "rect", val: { x: this.column * 36, y: 0, width: 36, height: 36, fill: 0x1e88e5 } }],
      value: `Column ${this.column + 1}`,
    };
  }
  async handleEvent(event: ViewEvent) {
    if (event.tag === "key" && event.val === "right") this.column = Math.min(this.column + 1, 7);
    if (event.tag === "key" && event.val === "left") this.column = Math.max(this.column - 1, 0);
    if (event.tag === "pointer-down") this.column = Math.min(Math.max(Math.floor(event.val.x / 36), 0), 7);
  }
}
// items: [{ id: "pick", title: "Pick", customView: { title: "Pick", label: "Column", role: "color-well" } }]

async openView(itemId) {
  return new Picker();
},
```

Both methods must be `async` in JS/TS (see the
[contract notes](../docs/custom-views.md#contract)); `@pane/extension` types
them as returning a `Promise`, so the build's type check rejects a
synchronous one. A frame may have at most 4096 shapes, 256 characters per
text and 4096 x 4096 pixels; Pane shows a larger one as your error. Throwing from
`handleEvent` shows the error and keeps the view; a crash closes it. A
command without custom views uses `type CustomView = NoCustomView;` in Rust
and makes `open_view`/`openView` fail.

## Operations

A package can publish operations, named and versioned functions other
extensions call through Pane with JSON input and results, and any command can
call another package's operations; the full contract, errors and limits are
in [docs/operations.md](../docs/operations.md). The operations samples show
both sides in [Rust](sample-operations/src/lib.rs),
[JavaScript](sample-operations-js/src/index.js) and
[TypeScript](sample-operations-ts/src/index.ts).

Publish in `pane.json`; only listed operations are callable:

```json
"operations": [{ "id": "greet", "version": 1, "component": "sample_operations.wasm" }]
```

The component named there serves them, beside its command, like a command
computing [root results](#root-results-computed-from-the-query). In Rust it
implements `pane_guest::publish::Guest` and calls
`pane_guest::publish::export!`:

```rust
pane_guest::export!(Greeter);
pane_guest::publish::export!(Greeter);

impl pane_guest::publish::Guest for Greeter {
    async fn run_operation(operation: String, input: String) -> Result<String, String> {
        // `input` and the returned text are JSON; `Err` is the operation's own error.
    }
}
```

A JavaScript or TypeScript package sets `"pane": { "operations": true }` in
its `package.json` and exports `publishedOperations` (typed as
`PublishedOperations`) with `async runOperation(operation, input)`; throwing
is the operation's own error.

Call another package's operation with its source, the operation, the version
you were written for and JSON input:

```rust
use pane_guest::operations::call;

let result = call(source.into(), "greet".into(), 1, input) // "local:/…/sample-operations-js"
    .await
    .map_err(|error| error.explain())?; // "not-found: …", "failed: …"
```

```ts
import { call, type CallError } from "pane:extension/operations@0.1.0";

try {
  const result = await call(source, "greet", 1, JSON.stringify({ name })); // "local:/…"
} catch (error) {
  const { kind, message } = (error as { payload: CallError }).payload;
}
```

`source` is the target's identity exactly as installed: `local:` and the
absolute folder path it was installed from, the path Manage extensions shows
after "local folder" (the samples ask for it in their form), or the id of a
dependency your `pane.json` declares ([below](#dependencies-on-other-extensions)),
which is how a package names the extensions it is written for. Pane starts the target only when it is called, never enables a disabled one,
keeps each package's settings apart, and refuses a call back into a package
already waiting in the same chain instead of deadlocking.

### Dependencies on other extensions

A package that calls other packages' operations declares them, so that
installing it installs what it needs
([details](../docs/dependencies.md)):

```json
"dependencies": [
  {
    "id": "greeter",
    "source": "local:../sample-operations-js",
    "operations": [{ "id": "greet", "version": 1 }]
  },
  {
    "id": "rust-greeter",
    "source": "local:../sample-operations",
    "optional": true,
    "operations": [{ "id": "greet", "version": 1 }]
  }
]
```

- `id`: the name your code calls it by, `call("greeter", "greet", 1, input)`,
  in place of its identity; lowercase letters, digits and `-`.
- `source`: `local:` and its folder, relative to your package's folder (the
  folder a link to it points to) or absolute, with `/` between folders on
  every system: `\`, drive letters and `//server` shares are refused. Pane
  resolves it as it resolves an installed folder and keeps what it resolved
  to, so moving your source folder later does not change it. Or `npm:` and
  an npm package name, optionally with an exact version
  (`npm:@pane-samples/greeter@0.1.0`), which Pane downloads when it is
  missing ([npm](../docs/npm.md#dependencies-from-npm)); a version pins it,
  so an installed copy of another version is a conflict Pane explains
  rather than a version your package did not ask for. A package you publish
  to npm can only use `npm:` sources. Git is not supported yet.
- `optional` (default `false`): a required dependency is installed with your
  package when it is missing; an optional one never is, and a call to it
  when it is not installed is `not-found` (the
  [sample](sample-dependencies/src/lib.rs) answers how to get it instead).
- `operations`: every operation you call, at the version you call. Pane
  checks them before installing anything, and a call through the id reaches
  only these.
- `platforms` (optional): the systems you need it on; elsewhere it is
  neither installed nor checked.

The preview lists each dependency: "Requires: <title>, installed with it
from <source>", "already installed", or disabled (it stays disabled), and
the optional ones. A required dependency that cannot be installed (missing
folder, source-only, other system, an operation it does not publish at that
version, two packages needing different versions of one operation) is
explained and nothing is installed. An installed dependency is never
replaced by installing another package: update it yourself.

## Native helpers

For what a WASI guest cannot do (an operating-system API, a native
library), a package can ship a **native helper**: an ordinary program built
for each operating system and processor it supports, which its commands run
through Pane. The command stays a WASI 0.3 component; Pane runs the helper's
file for the system it runs on and compiles nothing. The contract, errors
and limits are in [docs/helpers.md](../docs/helpers.md). The helper samples
are a [Rust](sample-helper/src/lib.rs), a
[JavaScript](sample-helper-js/src/index.js) and a
[TypeScript](sample-helper-ts/src/index.ts) command with their packages
([`packages/sample-helper`](packages/sample-helper/pane.json) and the
`-js` and `-ts` ones) and one helper,
[`helpers/echo`](helpers/echo/src/main.rs).

1. **Write the helper** as a plain program: it reads its input from
   standard input (closed after the input), writes its answer as UTF-8 text
   to standard output and exits with code 0; on failure it exits with
   another code and explains on standard error, which Pane shows. It may
   take arguments. It runs in its own folder of the installed copy, with
   Pane's environment, and must not leave processes behind: Pane ends the
   helper's own process when the run is cancelled or the package stops, not
   processes it started. It must be a native program: Pane refuses scripts
   (`#!`) on every system, and a Windows helper must be an `.exe`.
2. **Build it for each target** you support, on that system or with a
   cross toolchain, for example with Cargo:
   `cargo build --release --target aarch64-apple-darwin`. A target is
   `<os>-<arch>`: `windows`, `macos` or `linux`, then `x86_64` or `aarch64`.
   Link what it needs statically where you can: Pane does not check a
   helper's library dependencies or minimum OS version.
3. **Put the files in the package** (regular files, not symbolic links)
   and declare them in `pane.json`; an `id` is lowercase letters, digits
   and dashes:

   ```json
   "helpers": [
     {
       "id": "echo",
       "targets": {
         "linux-x86_64": "helpers/linux-x86_64/pane-echo",
         "macos-aarch64": "helpers/macos-aarch64/pane-echo",
         "windows-x86_64": "helpers/windows-x86_64/pane-echo.exe"
       }
     }
   ]
   ```

   Installing checks this system's file (it must exist and be a program for
   this system: 64-bit ELF on Linux, Mach-O on macOS, PE on Windows, for
   the named processor) and copies only it, with mode 0755; a package without a file
   for this system still installs, and running that helper explains the
   targets it has. The preview lists each helper's targets. For the sample,
   `cargo xtask guests` builds `pane-echo` for the system it runs on and puts
   it in `target/guests/packages/sample-helper/helpers/<target>/`.
4. **Run it from a command** by its `id`, with arguments and input:

   ```rust
   use pane_guest::helpers;

   let answer = helpers::run("echo".into(), vec![], "hello".into())
       .await
       .map_err(|error| format!("{}: {}", error.kind.name(), error.message))?;
   // "not-found: …", "unavailable: Not available on Linux arm64: …",
   // "failed: helper `echo` failed (exit code 3): …", "refused: …"
   ```

   Dropping the future before it resolves cancels the run, and Pane ends
   the process; the sample's "Echo within a second" races it against
   `wasip3::clocks::monotonic_clock::wait_for`. A helper also ends when the
   call that started it returns and when the package is disabled, reloaded,
   updated, paused or uninstalled, and when Pane quits.

   In JavaScript or TypeScript, import `run` from
   `pane:extension/helpers@0.1.0` (declared in
   [`js/helpers.d.ts`](js/helpers.d.ts)); a failed run rejects with the
   error as `payload`:

   ```ts
   import { run, type HelperError } from "pane:extension/helpers@0.1.0";

   try {
     return await run("echo", [], "hello");
   } catch (error) {
     const { kind, message } = (error as { payload: HelperError }).payload;
     throw new Error(`${kind}: ${message}`);
   }
   ```

   A promise cannot be cancelled: a run the command stops awaiting (the
   samples' `Promise.race` against `waitFor`) keeps its helper until the
   Pane call returns, which ends it.

## Packaging and installing a local extension

A package is a folder with a `pane.json` manifest at its root and the built
component of each command it lists. The same format serves Rust, JavaScript
and TypeScript: Pane sees only components.

```json
{
  "manifestVersion": 1,
  "title": "Hello",
  "version": "1.0.0",
  "apiVersion": "0.1",
  "commands": [
    {
      "id": "hello",
      "title": "Say hi",
      "subtitle": "Optional second line in root search",
      "component": "target/wasm32-wasip2/release/hello.wasm"
    }
  ]
}
```

- `manifestVersion` (required): the manifest format, currently `1`. A newer
  number is refused with "a newer Pane is needed".
- `title` (required): the display title. It is not the package's identity.
- `version` (optional): shown before installing and after an update.
- `apiVersion` (required): the `pane:extension` contract the components are
  built against, `MAJOR.MINOR` (this Pane provides `0.1`, from
  [`wit/extension.wit`](../wit/extension.wit)). Before 1.0 the minor version
  must match; from 1.0, any minor version up to Pane's in the same major.
- `platforms` (optional): the operating systems the package supports, from
  `windows`, `macos` and `linux`; omitted means all of them, and `[]` means
  none. On a system it does not list, Pane explains the package ("Not
  available on Linux: this package supports only Windows") instead of
  installing it. See
  [platform availability](../docs/platform-availability.md).
- `commands` (required, at least one): `id` unique in the package (without
  `#`, which Pane's records use to join it to the package identity), `title`,
  optional `subtitle`, optional `platforms` (the same list, for this command
  alone: elsewhere its root row is listed with the reason and does not
  open), and `component`, a relative path inside the package folder (no
  `..`, no absolute path) to a built component. Users find a command in
  root search by its `title` and its `subtitle` (the package `title` when it
  has none), so put the words people will type there; Pane searches this
  metadata without running the command
  ([root search](../docs/root-search.md#matching-and-ranking)). Optional
  `rootResults: true` says the command also computes
  [root results from the query](#root-results-computed-from-the-query).
- `operations` (optional): the [operations](#operations) the package
  publishes; `commands` may then be empty.
- `dependencies` (optional): the other packages whose operations it calls,
  required or optional ([dependencies](#dependencies-on-other-extensions)).
- `helpers` (optional): the [native helpers](#native-helpers) the package
  ships, each an `id` and its file for each target (`"linux-x86_64":
  "helpers/linux-x86_64/tool"`).

Unknown fields are ignored. The component must exist when you install: a
package whose component is not built is refused as source-only, with the
missing path. Pane then checks each component without running it: it must
compile, import only WASI 0.3 and export the extension interface, each
function Pane calls with the types it calls it with, and for a command with
`rootResults` the root results interface too.
A component built against an older shape of the same `apiVersion` (the
pre-release API 0.1 changes between slices) is therefore refused at install,
naming the first mismatch ("it was built for an older extension API shape:
rebuild it against Pane's current extension API 0.1 (`get-view`: type
mismatch for field items: expected record of 6 fields, found 4 fields)");
rebuild it against the current [`wit/extension.wit`](../wit/extension.wit).

Where the component comes from is up to your build. A standalone Rust crate
can point `component` at `target/wasm32-wasip2/release/<name>.wasm` inside
the crate folder after `cargo build --release --target wasm32-wasip2`. A
JavaScript or TypeScript package can build into its own folder with
`python3 tools/componentize-js/pane_js.py build <package dir> <package dir>/dist/<name>.wasm`
and point at `dist/<name>.wasm`. The repository's samples live in one Cargo
workspace and a prebuilt folder, so their manifests are in
[`packages/`](packages) and `cargo xtask guests` assembles each with its
component into `target/guests/packages/<name>/`.

To install, choose **Install extension from folder…** at the end of root
search, pick the package folder, check the source, version, commands and
compatibility Pane shows, and press Enter on **Install**. The package's
commands appear in root search, the first one selected. From the command line,
`cargo run -p pane -- --install target/guests/packages/sample-rust` (or
`pane --install <folder>`) opens the same screen, which also helps where no
folder picker is available: on Linux the picker is the desktop portal
(`xdg-desktop-portal`), and without one Pane shows why it could not open it.

What installing does:

- **Identity.** The package is identified by its folder's absolute path as
  the operating system resolves it (`std::fs::canonicalize`): symbolic links
  and `..` are followed, and on file systems that ignore letter case or
  Unicode normalization (the defaults on Windows and macOS) the stored
  spelling is used, so two spellings of one folder are one package. Pane does
  no case folding or normalization of its own, so on a case-sensitive Linux
  file system `Hello` and `hello` are two packages. On Windows the `\\?\`
  prefix is dropped. A folder path that is not valid Unicode is refused.
  Moving or renaming the folder makes it a different package.
- **Copy.** `pane.json` and the listed components (nothing else) are copied
  into Pane's data folder, under `extensions/packages/<n>/`, and recorded in
  `extensions/installed.json`. Your folder is never written, and the
  installed copy keeps working if the folder changes or is deleted. The data
  folder is `%LOCALAPPDATA%\Pane\data` on Windows,
  `~/Library/Application Support/Pane` on macOS and `$XDG_DATA_HOME/pane`
  (default `~/.local/share/pane`) on Linux; `PANE_DATA_DIR` overrides it.
- **Duplicates and updates.** Installing a folder that is already installed is
  refused. Choosing it again shows **Update** instead, which replaces the
  installed copy with the folder's current contents under the same identity,
  whatever its new title or version. Two different folders are two packages,
  even with identical contents, and nothing is merged or switched between
  them.
- **Required dependencies.** Installing or updating also installs the
  missing [required dependencies](#dependencies-on-other-extensions) the
  manifest declares, first, or explains why it cannot and installs nothing.
- **Listing.** Installed commands are listed from the manifests alone; no
  guest runs until you open a command or another extension calls one of the
  package's [operations](#operations). A damaged installed copy stays listed
  with its problem.
- **Disabling.** **Manage extensions…**, the last row of root search once a
  package is installed, lists every installed package with whether it is
  enabled and its source, so copies with the same title can be told apart.
  Enter disables or enables the selected one; only that installation
  changes. A disabled package's commands leave root search (they are not
  shown greyed out), an open command of it closes, its running instances are
  dropped and it can no longer save settings, so none of its code runs. This
  happens as soon as you press Enter, before the choice is written; if it
  cannot be written, the package is enabled again with the reason. Pressing
  Enter again while the choice is being written does nothing. The choice is
  recorded in
  `installed.json` (`"disabled": true`) and holds after restarting Pane and
  after an Update. Its settings are kept, and enabling it brings its
  commands back with them. The package stays installed at the same identity;
  choosing its folder again shows it as disabled.
- **Uninstalling.** **Uninstall <title>** in Manage extensions asks first,
  showing how many settings and content records the package keeps, and
  offers **Uninstall and keep saved data**, **Uninstall and delete saved
  data** or **Cancel**. Either way Pane removes the installed copy, the
  package's cache and its local credentials, without running it, and leaves
  its source folder and anything outside Pane's data folder alone. Kept
  settings and content stay with the source identity: installing the same
  folder again finds them; another folder never does
  ([details](../docs/extension-data.md#uninstalling-an-extension)).

### Reloading a package while Pane stays open

After rebuilding a component, reload the package instead of restarting
Pane: in **Manage extensions…**, after the rows that enable or disable each
package, every enabled package has a **Reload <title>** row. Enter (or a
click) on it reads the package's source folder again and replaces only that
package; Pane and every other package keep running, including a custom view
of another package that is open. It works the same for Rust, JavaScript and
TypeScript packages, since Pane sees only components. A reload goes through
two stages, and a failure in each is reported differently:

1. **Checks.** The folder is checked exactly as an install checks it
   (manifest, built components, WASI 0.3 imports, the extension API shape),
   without running anything. If that fails, nothing is replaced: the
   package keeps running the code installed before, and the status says
   "Dev was not reloaded: <reason>. It keeps running its installed code."
2. **Start.** Otherwise the new copy replaces the installed one, the old
   instances are stopped (an open command, form or custom view of the
   package closes; root search then selects its command), and the new code
   starts: Pane starts each of the package's commands available on this
   system and asks it for its view (`get-view`). Success shows "Reloaded
   Dev". If a command fails to initialize (it traps, or its component
   cannot load or be instantiated), its instances are stopped again and the
   package is reported as failed to start. An error the command returns
   from `get-view` itself, such as asking the user to sign in first, is an
   ordinary answer and not a failure to start. On a failure to start: the package's row says "Failed to start", a **Retry
   starting <title>** row appears under its Reload row with the diagnostics
   (for a trap, the guest backtrace), which Pane also writes to its standard
   error. The earlier code is not restored. Retry starts the same code
   again; to fix it, rebuild and reload.

What a reload keeps and what it does not:

- **Settings are kept.** They belong to the package identity, so the new
  code reads what the old code saved ([Keeping settings](#keeping-settings)),
  including anything saved by a start that then failed. Nothing is migrated
  or undone.
- **Nothing live is carried over.** The old instance's memory, an open
  view's state and a running call are not transferred to the new code, and
  there is no API for an extension to hand transient state to its
  replacement: save what must survive in settings. An answer from the old
  code that arrives after the reload (for example a command that was
  opening) is not shown.
- A disabled package has no Reload row and is not reloaded; enable it first.
- Reload is also what [development mode](#developing-a-package-build-and-reload-on-save)
  does after each save that builds. The Update in the install screen still
  replaces the copy too, without the start stage.

### Developing a package: build and reload on save

Instead of rebuilding and pressing Reload after each change, choose
**Develop <title>** in **Manage extensions…** (the last rows, one per
enabled package). Pane then watches the package's source folder and, after
each save, runs its build there and reloads the package when the build
succeeds:

- A folder with `Cargo.toml` is built with `cargo build --release --target
  wasm32-wasip2` (with cargo's JSON messages, which say where it built the
  component), so `pane.json` names its component under
  `target/wasm32-wasip2/release/`; Pane takes the file of that name cargo
  built this time, even with another target folder.
- A folder with `package.json` is built with
  `python3 tools/componentize-js/pane_js.py build <folder> <out>` for each
  component `pane.json` names, such as `dist/<name>.wasm` (a Pane run from a
  checkout knows where `pane_js.py` is; otherwise set
  `PANE_COMPONENTIZE_JS`; `PANE_PYTHON` names the interpreter).

Each build puts the components in a staging folder under Pane's data
folder, and runs with Pane's environment (less what `cargo run` set for
Pane itself). Once development is on, any write to the folder, such as
`git pull` or an autosave, runs the build, `build.rs` included.

A build that fails replaces nothing: the command keeps running its installed
code, the status line shows the first error, and **Why <title> did not
build** shows the end of the build's output and the path of a log file with
all of it. A build that succeeds is reloaded from its staging folder as
**Reload <title>** does, including a failure to start, which pauses the
package with Retry and does not restore the earlier code; the next save that
builds recovers it; its components are then copied where `pane.json` names
them. Saving again while a build runs makes that build obsolete: it is never
reloaded, and the folder is built again (after three in a row, Pane waits
for the next save). **Stop developing <title>**, disabling or uninstalling
the package, or quitting Pane ends it and kills a running build with the
processes it started. Only that installation is
affected: a copy of the package installed from another folder keeps its own
code. The `hello-rust`, `hello-js` and `hello-ts` samples are ready to try;
[development mode](../docs/development-mode.md) has the steps, what is
watched and the limits.

## Publishing a package to npm

A Pane package can be published to npm, so that users install it by name
with **Install extension from npm…** (or `pane --install npm:<name>`),
without Node.js or npm ([details](../docs/npm.md)). Pane installs the
package's tarball exactly as it installs a folder, and runs nothing else in
it: publish what is **built**.

1. **Build first.** Build every component `pane.json` names, and each native
   helper for every target you support, before packing: Pane never runs npm
   install scripts (`preinstall`, `install`, `postinstall`, `prepare`…) and
   never installs your npm `dependencies`, so a component bundling a library
   must have it built in. A package published without its components is
   explained to users as source-only.
2. **Add a `package.json`** beside `pane.json`, whose `files` lists
   `pane.json` and what it names, and nothing Pane would not use:

   ```json
   {
     "name": "@your-scope/your-extension",
     "version": "1.0.0",
     "description": "…",
     "license": "…",
     "keywords": ["pane-extension"],
     "files": ["pane.json", "dist/command.wasm", "helpers/"]
   }
   ```

   Its `name` is the package's identity in Pane (every version is the same
   package), and each version you publish is what users install by
   `name@version` or as the latest. Keep `pane.json` at the package's root.
   Its dependencies on other Pane packages are `npm:` sources.
3. **Check the tarball** with `npm pack --dry-run`: it lists what users
   will download. Helpers keep their files; Pane sets their mode itself.
   Symbolic links, and names some system reads differently or cannot write
   (`\ : < > " | ? *`, a name ending in `.` or a space, `con`, `nul`,
   `com1`…, on every system alike), make Pane refuse the whole tarball.
4. **Try it before publishing**: `npm pack` makes the `.tgz`; serve it from
   a registry on this computer and point a development build of Pane at it
   with `PANE_NPM_REGISTRY=http://127.0.0.1:<port>/`
   ([`scripts/npm_registry.py`](../scripts/npm_registry.py) serves the
   tarballs of a folder), then install it by name.
5. **Publish** with `npm publish` (`--access public` for a scoped name).
   Pane needs the registry's sha512 integrity, which npm gives every
   version it publishes.

The sample [`npm/greeter`](npm/greeter) is such a package, kept
`"private": true` so that it is never published. `cargo xtask guests`
assembles and packs it on each contributor system, without npm; `npm pack`
in the assembled `target/guests/npm/greeter` makes the same list of files,
and its tarball installs alike.

Uninstalling is not implemented yet.

Known limits of local packages so far:

- An update is not coordinated with a command that is open: the replaced
  copy's code is dropped and its pending calls stop, so an open command of
  the package loses its state.
- Disabling, reloading or updating a package **stops its pending calls**
  (see [generations](../docs/generations.md)): a call waiting inside the
  command at an `await` (a clock, an operation, any async import) ends
  there, nothing after that `await` runs, the instance is dropped and the
  answer is never shown; a call that had not started is not started. The
  settings samples' "Save after waiting" shows it: it saves "started",
  waits ten seconds, then saves "finished", which a disable or reload
  meanwhile prevents. So save what must survive before awaiting, and do
  not count on code after an `await` running. A command computing without
  awaiting is not interrupted: it runs until it awaits or returns, and
  meanwhile Pane refuses it data, operation calls and applications (#18
  owns hangs).
- Background services, timers and hotkeys are not part of the extension
  API yet and come with their own tickets. Disabling does not yet consider
  packages that depend on the disabled one (#43).
