#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Build Pane's JavaScript and TypeScript commands into WASI 0.3 components.

Usage:
  pane_js.py toolchain                  fetch and build the pinned toolchain
  pane_js.py build <package> <out.wasm> type-check, bundle and componentize one command
  pane_js.py samples                    rebuild guests/prebuilt/ and its manifest
  pane_js.py check                      verify guests/prebuilt/ against its manifest
                                        and the current sources (needs no toolchain)

The toolchain is upstream componentize-qjs at a pinned commit plus the patch
queue in patches/, built with a pinned Rust nightly and wasi-sdk (pins.json),
and esbuild/TypeScript from package-lock.json. Downloads and builds are cached
in PANE_JS_TOOLCHAIN_DIR, by default the user cache directory
(pane/componentize-js). Nothing outside that directory and the output paths is
written, except that rustup installs the pinned toolchains.

Prerequisites on every OS: Python 3.12+, git, Node.js 22+ with npm, and rustup.
"""
from __future__ import annotations

import hashlib
import json
import os
import platform
import re
import shlex
import shutil
import subprocess
import sys
import tarfile
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
PINS = json.loads((HERE / "pins.json").read_text(encoding="utf-8"))
EXE = ".exe" if os.name == "nt" else ""
WORLD = "js-extension"
# The world a command is built against: `js-extension` plus the exports its
# package.json's `"pane"` options name, written by `command_world`.
COMMAND_WORLD = "js-command"
# `"pane"` option -> the interface a command setting it also exports.
EXPORT_OPTIONS = {
    "rootResults": "pane:extension/root-results@0.1.0",
    "indexedResults": "pane:extension/indexed-results@0.1.0",
    "operations": "pane:extension/published-operations@0.1.0",
    "takesQuery": "pane:extension/query-command@0.1.0",
    "search": "pane:extension/command-search@0.1.0",
    "scheduledTask": "pane:extension/scheduled-task@0.1.0",
    "service": "pane:extension/service@0.1.0",
}
# `"pane"` option -> the interface a command setting it also imports, beyond
# what every command may import (`js-extension`): a command that sets none
# of them does not import it at all.
IMPORT_OPTIONS = {
    "files": "pane:extension/files@0.1.0",
}
PREBUILT = REPO / "guests" / "prebuilt"
MANIFEST = PREBUILT / "manifest.json"
# (component file in guests/prebuilt and target/guests, source package)
SAMPLES = [
    ("sample_js.wasm", "guests/sample-js"),
    ("sample_ts.wasm", "guests/sample-ts"),
    ("sample_settings_js.wasm", "guests/sample-settings-js"),
    ("sample_settings_ts.wasm", "guests/sample-settings-ts"),
    ("sample_operations_js.wasm", "guests/sample-operations-js"),
    ("sample_operations_ts.wasm", "guests/sample-operations-ts"),
    ("sample_applications_js.wasm", "guests/sample-applications-js"),
    ("sample_applications_ts.wasm", "guests/sample-applications-ts"),
    ("sample_query_js.wasm", "guests/sample-query-js"),
    ("sample_query_ts.wasm", "guests/sample-query-ts"),
    ("sample_search_js.wasm", "guests/sample-search-js"),
    ("sample_search_ts.wasm", "guests/sample-search-ts"),
    ("sample_helper_js.wasm", "guests/sample-helper-js"),
    ("sample_helper_ts.wasm", "guests/sample-helper-ts"),
    ("sample_files_js.wasm", "guests/sample-files-js"),
    ("sample_files_ts.wasm", "guests/sample-files-ts"),
    ("sample_npm_js.wasm", "guests/sample-npm-js"),
    ("sample_background_js.wasm", "guests/sample-background-js"),
    ("sample_background_ts.wasm", "guests/sample-background-ts"),
    ("sample_service_js.wasm", "guests/sample-service-js"),
    ("sample_service_ts.wasm", "guests/sample-service-ts"),
]
# Pane's WIT, copied beside the world in guests/js/wit.
PANE_WIT = ["extension.wit", "data.wit", "root-results.wit", "operations.wit", "applications.wit", "query.wit",
            "search.wit", "helpers.wit", "files.wit", "background.wit"]
# WASI's WIT (clocks, and `wasi:http` with the packages it names), copied from
# wit/deps into the world's deps/.
WASI_WIT = sorted((REPO / "wit" / "deps").glob("*.wit"))
# Toolchain inputs that decide what a component contains.
TOOL_INPUTS = ["pins.json", "package.json", "package-lock.json", "bundle.mjs", "p3_build.rs", "patches"]
SKIP_DIRS = {"node_modules", ".git"}


def cache_root() -> Path:
    configured = os.environ.get("PANE_JS_TOOLCHAIN_DIR")
    if configured:
        return Path(configured).resolve()
    if sys.platform == "win32":
        base = Path(os.environ.get("LOCALAPPDATA", Path.home() / "AppData" / "Local"))
    elif sys.platform == "darwin":
        base = Path.home() / "Library" / "Caches"
    else:
        base = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache"))
    return base / "pane" / "componentize-js"


CACHE = cache_root()


def log(message: str) -> None:
    print(f"pane-js: {message}", flush=True)


def tool(name: str) -> str:
    found = shutil.which(name)
    if not found:
        raise SystemExit(f"pane-js: `{name}` was not found on PATH; see guests/README.md for prerequisites")
    return found


def run(cmd, *, cwd=None, env=None, capture=False) -> str:
    cmd = [str(part) for part in cmd]
    result = subprocess.run(cmd, cwd=cwd, env=env, text=True,
                            stdout=subprocess.PIPE if capture else None)
    if result.returncode:
        raise SystemExit(f"pane-js: {' '.join(cmd)} failed with exit code {result.returncode}")
    return result.stdout or ""


def clean_env(**extra: str) -> dict[str, str]:
    """The caller's environment without Cargo/rustup settings inherited from `cargo xtask`."""
    env = {key: value for key, value in os.environ.items()
           if not (key.startswith("CARGO_") and key != "CARGO_HOME")
           and key not in {"CARGO", "RUSTUP_TOOLCHAIN", "RUSTC", "RUSTDOC", "RUSTFLAGS"}}
    env.update(extra)
    return env


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def tree_files(root: Path) -> list[Path]:
    if root.is_file():
        return [root]
    files = []
    for directory, dirs, names in os.walk(root):
        dirs[:] = sorted(d for d in dirs if d not in SKIP_DIRS)
        files += [Path(directory) / name for name in names]
    return sorted(files)


def inputs_digest(paths: list[Path]) -> str:
    """Digest of text inputs by repository-relative path, with line endings normalized."""
    digest = hashlib.sha256()
    for root in paths:
        for path in tree_files(root):
            digest.update(path.relative_to(REPO).as_posix().encode() + b"\0")
            digest.update(path.read_bytes().replace(b"\r\n", b"\n") + b"\0")
    return digest.hexdigest()


def tool_inputs() -> list[Path]:
    return [HERE / name for name in TOOL_INPUTS]


def host_platform() -> tuple[str, str]:
    """(arch, os) as wasi-sdk release assets spell them."""
    machine = platform.machine().lower()
    arch = {"amd64": "x86_64", "x86_64": "x86_64", "arm64": "arm64", "aarch64": "arm64"}.get(machine)
    system = {"linux": "linux", "darwin": "macos", "win32": "windows"}.get(sys.platform)
    if arch is None or system is None:
        raise SystemExit(f"pane-js: no wasi-sdk build for {sys.platform}/{machine}; "
                         "supported hosts are x86_64 and arm64 Linux, macOS and Windows")
    return arch, system


def download(url: str, path: Path, expected: str) -> None:
    if not path.exists() or sha256_file(path) != expected:
        log(f"downloading {url}")
        path.parent.mkdir(parents=True, exist_ok=True)
        partial = path.with_suffix(path.suffix + ".part")
        with urllib.request.urlopen(url) as response, partial.open("wb") as out:
            shutil.copyfileobj(response, out)
        partial.replace(path)
    actual = sha256_file(path)
    if actual != expected:
        raise SystemExit(f"pane-js: {path.name} has sha256 {actual}, expected {expected}")


def extract(archive: Path, into: Path) -> Path:
    """Extracts a tarball with one top-level directory into `into`; returns that directory."""
    into.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive) as tar:
        top = tar.getmembers()[0].name.split("/")[0]
        tar.extractall(into, filter="tar")
    return into / top


def rust_stable() -> str:
    """The repository's pinned stable toolchain, which builds the componentizer."""
    text = (REPO / "rust-toolchain.toml").read_text(encoding="utf-8")
    return re.search(r'^channel\s*=\s*"([^"]+)"', text, re.M).group(1)


class Toolchain:
    def __init__(self) -> None:
        arch, system = host_platform()
        sdk = PINS["wasi_sdk"]
        self.sdk_name = f"wasi-sdk-{sdk['version']}-{arch}-{system}"
        self.sdk_digest = sdk["archive_sha256"][f"{arch}-{system}"]
        self.sdk = CACHE / self.sdk_name
        self.key = inputs_digest(tool_inputs())[:16]
        qjs = PINS["componentize_qjs"]
        self.source = CACHE / "src" / f"componentize-qjs-{qjs['commit'][:12]}-{self.key}"
        self.bin = CACHE / "bin" / self.key
        self.runtime = self.bin / "runtime.wasm"
        self.componentizer = self.bin / f"componentize-qjs-p3{EXE}"
        self.node = CACHE / "node"
        self.stamp = self.bin / "toolchain.json"

    @property
    def libc(self) -> Path:
        return self.sdk / "share" / "wasi-sysroot" / "lib" / "wasm32-wasip3" / "libc.so"

    def ensure(self) -> dict:
        """Makes the toolchain ready, building only what the cache lacks."""
        self.ensure_sdk()
        if not self.stamp.exists():
            self.build()
        self.ensure_node()
        return json.loads(self.stamp.read_text(encoding="utf-8"))

    def ensure_sdk(self) -> None:
        """wasi-sdk: its compiler builds the runtime; its P3 libc is linked into every component."""
        if self.libc.exists():
            return
        sdk = PINS["wasi_sdk"]
        archive = CACHE / "downloads" / f"{self.sdk_name}.tar.gz"
        download(f"https://github.com/WebAssembly/wasi-sdk/releases/download/{sdk['release']}/{self.sdk_name}.tar.gz",
                 archive, self.sdk_digest)
        extract(archive, CACHE)

    def build(self) -> None:
        log(f"building the componentize-qjs toolchain into {CACHE}")
        qjs, sdk = PINS["componentize_qjs"], PINS["wasi_sdk"]
        repo_path = qjs["repository"].removeprefix("https://github.com/")
        archive = CACHE / "downloads" / f"componentize-qjs-{qjs['commit']}.tar.gz"
        download(f"https://codeload.github.com/{repo_path}/tar.gz/{qjs['commit']}", archive, qjs["archive_sha256"])

        # A fresh checkout of the pinned source with the patch queue applied.
        if self.source.exists():
            shutil.rmtree(self.source)
        scratch = CACHE / "src" / "extract"
        shutil.rmtree(scratch, ignore_errors=True)
        extract(archive, scratch).rename(self.source)
        shutil.rmtree(scratch)
        git = tool("git")
        run([git, "init", "-q"], cwd=self.source)
        for patch in PINS["patches"]:
            run([git, "apply", "--whitespace=nowarn", HERE / "patches" / patch], cwd=self.source)
            log(f"applied {patch}")
        examples = self.source / "crates" / "core" / "examples"
        examples.mkdir(exist_ok=True)
        shutil.copyfile(HERE / "p3_build.rs", examples / "p3_build.rs")

        nightly, stable = PINS["rust_nightly"], rust_stable()
        rustup = tool("rustup")
        run([rustup, "toolchain", "install", nightly, "--profile", "minimal", "--component", "rust-src"])
        run([rustup, "toolchain", "install", stable, "--profile", "minimal"])

        # The QuickJS runtime, for wasm32-wasip3 against the SDK's P3 libc.
        clang = str(self.sdk / "bin" / f"clang{EXE}")
        sysroot_lib = self.libc.parent
        runtime_target = CACHE / "runtime-target"
        env = clean_env(
            CARGO_TARGET_DIR=str(runtime_target),
            PATH=str(self.sdk / "bin") + os.pathsep + os.environ["PATH"],
            CARGO_TARGET_WASM32_WASIP3_LINKER=clang,
            # Encoded (0x1f-separated) so a cache path containing spaces stays
            # one argument. With --target, these reach only the Wasm target.
            CARGO_ENCODED_RUSTFLAGS="\x1f".join([
                "-Crelocation-model=pic", "-Clink-arg=--target=wasm32-wasip3",
                "-Clink-arg=-shared", "-Clink-arg=-Wl,--no-entry", "-Clink-arg=-Wl,--allow-undefined",
                "-Clink-arg=-Wl,--export=__wasm_library_tls_info", "-L", f"native={sysroot_lib}"]),
            WASI_SDK=str(self.sdk), WASI_SDK_PATH=str(self.sdk),
            # rquickjs' bindgen loads the SDK's libclang (bin/ on Windows, lib/ elsewhere).
            LIBCLANG_PATH=str(self.sdk / ("bin" if os.name == "nt" else "lib")),
            CC_wasm32_wasip3=clang,
            AR_wasm32_wasip3=str(self.sdk / "bin" / f"llvm-ar{EXE}"),
            # rquickjs-sys would pass the sysroot through CFLAGS, which cc splits
            # on whitespace, so a cache path with spaces breaks it. The SDK's
            # clang finds its own sysroot; bindgen gets it shell-quoted instead.
            RQUICKJS_SYS_NO_WASI_SDK="1",
            BINDGEN_EXTRA_CLANG_ARGS_wasm32_wasip3=shlex.quote(f"--sysroot={self.sdk / 'share' / 'wasi-sysroot'}"),
            CFLAGS_wasm32_wasip3="--target=wasm32-wasip3 -fPIC -Oz",
        )
        run([rustup, "run", nightly, "cargo", "build", "--release", "--locked", "--target", "wasm32-wasip3",
             "-Zbuild-std=std,panic_abort", "--manifest-path", self.source / "Cargo.toml",
             "-p", "componentize-qjs-runtime"], env=env)
        self.bin.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(runtime_target / "wasm32-wasip3" / "release" / "componentize_qjs_runtime.wasm", self.runtime)

        # The componentizer. Its build script requires four prebuilt runtimes;
        # Pane always passes the runtime explicitly, so copies satisfy it.
        prebuilt = self.source / "crates" / "core" / "prebuilt"
        prebuilt.mkdir(exist_ok=True)
        for name in ["runtime.wasm", "runtime-opt-size.wasm", "runtime-sync.wasm", "runtime-opt-size-sync.wasm"]:
            shutil.copyfile(self.runtime, prebuilt / name)
        componentizer_target = CACHE / "componentizer-target"
        run([rustup, "run", stable, "cargo", "build", "--release", "--locked", "--manifest-path",
             self.source / "Cargo.toml", "-p", "componentize-qjs", "--example", "p3_build"],
            env=clean_env(CARGO_TARGET_DIR=str(componentizer_target)))
        shutil.copyfile(componentizer_target / "release" / "examples" / f"p3_build{EXE}", self.componentizer)
        self.componentizer.chmod(0o755)

        versions = {
            "componentize_qjs": {k: qjs[k] for k in ["repository", "version", "commit"]},
            "patches": {name: sha256_file(HERE / "patches" / name) for name in PINS["patches"]},
            "runtime_rustc": run([rustup, "run", nightly, "rustc", "-V"], capture=True).strip(),
            "componentizer_rustc": run([rustup, "run", stable, "rustc", "-V"], capture=True).strip(),
            "wasi_sdk": sdk["version"],
            "runtime_sha256": sha256_file(self.runtime),
            "built_on": "-".join(host_platform()),
        }
        self.stamp.write_text(json.dumps(versions, indent=2) + "\n", encoding="utf-8")

    def ensure_node(self) -> None:
        """esbuild and TypeScript at the versions in package-lock.json."""
        lock = HERE / "package-lock.json"
        marker = self.node / "node_modules" / ".pane-lock-sha256"
        if marker.exists() and marker.read_text() == sha256_file(lock):
            return
        self.node.mkdir(parents=True, exist_ok=True)
        for name in ["package.json", "package-lock.json"]:
            shutil.copyfile(HERE / name, self.node / name)
        npm_ci(self.node)
        marker.write_text(sha256_file(lock))

    def node_versions(self) -> dict[str, str]:
        lock = json.loads((HERE / "package-lock.json").read_text(encoding="utf-8"))["packages"]
        return {name: lock[f"node_modules/{name}"]["version"] for name in ["esbuild", "typescript"]}


def npm_ci(directory: Path) -> None:
    # Install scripts are not needed by these packages and are not run.
    run([tool("npm"), "ci", "--ignore-scripts", "--no-audit", "--no-fund"], cwd=directory)


def build(package: Path, out: Path, toolchain: Toolchain) -> dict:
    """Type-checks, bundles and componentizes the command package at `package`."""
    package = package.resolve()
    manifest = json.loads((package / "package.json").read_text(encoding="utf-8"))
    entry = manifest.get("main")
    if not entry:
        raise SystemExit(f"pane-js: {package / 'package.json'} needs a `main` entry naming the command module")
    # Stage the package beside Pane's types (`@pane/extension` is `file:../js`),
    # so dependencies install into the cache rather than the source tree.
    work = CACHE / "work" / f"{package.name}-{hashlib.sha256(str(package).encode()).hexdigest()[:8]}"
    staged, types = work / package.name, work / "js"
    shutil.rmtree(types, ignore_errors=True)
    shutil.copytree(REPO / "guests" / "js", types)
    if staged.exists():
        for child in staged.iterdir():
            if child.name != "node_modules":
                shutil.rmtree(child) if child.is_dir() else child.unlink()
    shutil.copytree(package, staged, dirs_exist_ok=True, ignore=shutil.ignore_patterns(*SKIP_DIRS))
    lock = staged / "package-lock.json"
    if lock.exists():
        marker = staged / "node_modules" / ".pane-lock-sha256"
        if not (marker.exists() and marker.read_text() == sha256_file(lock)):
            npm_ci(staged)
            marker.write_text(sha256_file(lock))

    node = tool("node")
    modules = toolchain.node / "node_modules"
    if (staged / "tsconfig.json").exists():
        log(f"type-checking {package.name}")
        # From the staged package, so errors name its files as the author
        # does ("src/index.ts(3,7): error ...").
        run([node, modules / "typescript" / "bin" / "tsc", "-p", staged / "tsconfig.json"], cwd=staged)
    bundle = work / "bundle.mjs"
    adapted = work / "pane-entry.mjs"
    adapted.write_text(adapted_entry(staged / entry, types / "adapt.js", manifest.get("pane", {})),
                       encoding="utf-8")
    run([node, HERE / "bundle.mjs", modules, adapted, bundle])

    wit = types / "wit"
    (wit / "deps" / "pane-extension").mkdir(parents=True, exist_ok=True)
    for name in PANE_WIT:
        shutil.copyfile(REPO / "wit" / name, wit / "deps" / "pane-extension" / name)
    for path in WASI_WIT:
        shutil.copyfile(path, wit / "deps" / path.name)
    out.parent.mkdir(parents=True, exist_ok=True)
    bundled = bundle.read_text(encoding="utf-8")
    world = command_world(manifest.get("pane", {}), uses_http(bundled), uses_status(bundled))
    (wit / "command.wit").write_text(world, encoding="utf-8")
    report = run([toolchain.componentizer, wit, COMMAND_WORLD, bundle, toolchain.runtime, out],
                 env=clean_env(QJS_P3_LIBC=str(toolchain.libc)), capture=True)
    result = json.loads(report.strip().splitlines()[-1])
    log(f"built {out} ({result['component_bytes']} bytes in {result['componentize_ms']} ms)")
    return result


# The exports the adapter wraps, by `pane` option (None: always), each with
# the handler that answers errors as text (see guests/js/adapt.js).
ADAPTED_PROVIDERS = {
    "rootResults": ("rootResults", "resultsFor"),
    "indexedResults": ("indexedResults", "results"),
    "operations": ("publishedOperations", "runOperation"),
    "takesQuery": ("queryCommand", "runQuery"),
    "search": ("commandSearch", "search"),
    "scheduledTask": ("scheduledTask", "runTask"),
    "service": ("service", "runService"),
}


def adapted_entry(entry: Path, adapter: Path, options: dict) -> str:
    """The module bundled for `entry`: its exports, with the ones Pane calls
    wrapped by `adapter` so that what a handler throws is an error, not a
    crash."""
    entry_js, adapter_js = json.dumps(entry.as_posix()), json.dumps(adapter.as_posix())
    lines = [
        f"import * as extension from {entry_js};",
        f"import {{ adaptCommand, adaptProvider }} from {adapter_js};",
        f"export * from {entry_js};",
        "export const command = adaptCommand(extension.command);",
    ]
    for option, (name, handler) in ADAPTED_PROVIDERS.items():
        if options.get(option):
            lines.append(f"export const {name} = adaptProvider(extension.{name}, {json.dumps(handler)});")
    return "\n".join(lines) + "\n"


# `wasi:http`'s client, which a command imports only if its bundle uses it
# (itself, or through `@pane/extension/http`), as a Rust command's component
# imports only what its code calls: Pane lists a package whose component
# imports it as one that uses the network.
HTTP_IMPORT = "wasi:http/client@0.3.0"


def uses_http(bundle: str) -> bool:
    """Whether the bundled module imports any `wasi:http` interface."""
    return re.search(r"""(?:from|import)\s*\(?\s*["']wasi:http/""", bundle) is not None


# A running service's status, which a command imports only if its bundle
# uses it, so that adding it changed no other command's component.
STATUS_IMPORT = "pane:extension/service-status@0.1.0"


def uses_status(bundle: str) -> bool:
    """Whether the bundled module imports the service status interface."""
    return re.search(r"""(?:from|import)\s*\(?\s*["']pane:extension/service-status@""", bundle) is not None


def command_world(options: dict, http: bool, status: bool = False) -> str:
    """The world `js-command`: `js-extension` exporting and importing what
    `options` name, importing `wasi:http`'s client if `http` and the
    service status if `status`."""
    unknown = sorted(set(options) - set(EXPORT_OPTIONS) - set(IMPORT_OPTIONS))
    if unknown:
        raise SystemExit(f"pane-js: unknown \"pane\" options in package.json: {', '.join(unknown)}")
    exports = "".join(f"  export {interface};\n" for option, interface in EXPORT_OPTIONS.items()
                      if options.get(option))
    imports = "".join(f"  import {interface};\n" for option, interface in IMPORT_OPTIONS.items()
                      if options.get(option))
    if http:
        imports += f"  import {HTTP_IMPORT};\n"
    if status:
        imports += f"  import {STATUS_IMPORT};\n"
    return (f"package pane:js-guest@0.1.0;\n\nworld {COMMAND_WORLD} {{\n  include {WORLD};\n"
            f"{imports}{exports}}}\n")


def component_inputs(source: str) -> str:
    pane_wit = [REPO / "wit" / name for name in PANE_WIT] + WASI_WIT
    return inputs_digest(tool_inputs() + pane_wit + [REPO / "guests" / "js", REPO / source])


def samples() -> None:
    toolchain = Toolchain()
    versions = toolchain.ensure()
    components = {}
    for name, source in SAMPLES:
        out = PREBUILT / name
        build(REPO / source, out, toolchain)
        components[name] = {
            "source": source,
            "inputs_sha256": component_inputs(source),
            "bytes": out.stat().st_size,
            "sha256": sha256_file(out),
        }
    manifest = {
        "about": ("JS/TS sample components used by tests and `cargo run -p pane`; rebuild with "
                  "`cargo xtask js-guests`. Rebuilds are not byte-identical: the QuickJS snapshot "
                  "holds build-time state. inputs_sha256 covers the sources and toolchain pins."),
        "toolchain": {**versions, **toolchain.node_versions()},
        "components": components,
    }
    MANIFEST.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    log(f"wrote {MANIFEST}")


def check() -> None:
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    problems = []
    for name, source in SAMPLES:
        entry = manifest["components"].get(name)
        path = PREBUILT / name
        if entry is None or not path.exists():
            problems.append(f"{name} is missing")
            continue
        if sha256_file(path) != entry["sha256"]:
            problems.append(f"{name} does not match its manifest sha256")
        if component_inputs(source) != entry["inputs_sha256"]:
            problems.append(f"{name} is stale: {source} or the toolchain pins changed since it was built")
    if problems:
        raise SystemExit("pane-js: " + "; ".join(problems) + ". Run `cargo xtask js-guests`.")
    log("prebuilt components match their sources")
    check_npm_licenses()


# npm packages bundled into components must keep them permissively licensed,
# like the Cargo policy in guests/deny.toml.
NPM_LICENSES = {"MIT", "MIT-0", "Apache-2.0", "Apache-2.0 OR MIT", "MIT OR Apache-2.0", "BSD-2-Clause",
                "BSD-3-Clause", "ISC", "0BSD", "Zlib", "CC0-1.0", "Unlicense"}


def check_npm_licenses() -> None:
    problems = []
    for _, source in SAMPLES:
        lock = json.loads((REPO / source / "package-lock.json").read_text(encoding="utf-8"))
        for path, package in lock.get("packages", {}).items():
            if package.get("link") or path.startswith("..") or not path:
                continue  # in-repo packages carry the repository's own license
            if not package.get("dev") and package.get("license") not in NPM_LICENSES:
                problems.append(f"{source}: {path} is licensed {package.get('license')!r}")
    if problems:
        raise SystemExit("pane-js: bundled npm packages need a license review: " + "; ".join(problems))
    log("bundled npm packages are permissively licensed")


def main(argv: list[str]) -> None:
    match argv:
        case ["toolchain"]:
            Toolchain().ensure()
            log(f"toolchain ready in {CACHE}")
        case ["build", package, out]:
            toolchain = Toolchain()
            toolchain.ensure()
            build(Path(package), Path(out).resolve(), toolchain)
        case ["samples"]:
            samples()
        case ["check"]:
            check()
        case _:
            raise SystemExit(__doc__)


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except SystemExit as failure:
        # A failure is one line starting "pane-js: error:" on standard error,
        # which Pane's development mode shows as a build's first error.
        message = failure.code
        if isinstance(message, str) and message.startswith("pane-js: "):
            print("pane-js: error: " + message.removeprefix("pane-js: "), file=sys.stderr, flush=True)
            raise SystemExit(1) from None
        raise
