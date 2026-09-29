//! Portable developer commands, run as `cargo xtask <command>`.
//!
//! - `guests`: build the Rust guests and copy them, with the prebuilt JS/TS
//!   sample components from `guests/prebuilt/`, into `target/guests/`, and
//!   assemble the sample packages into `target/guests/packages/`, with the
//!   helper sample's native helper built for this system, and the npm
//!   sample into `target/guests/npm/`, packed as npm packs it.
//! - `js-guests`: rebuild the prebuilt JS/TS sample components with the pinned
//!   toolchain in `tools/componentize-js` (prerequisites: guests/README.md),
//!   then run `guests`.
//! - `ci`: build guests, then check formatting, lints and tests.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const GUEST_TARGET: &str = "wasm32-wasip2";

/// Components built by `js-guests` and committed, so that normal builds and
/// tests need no JavaScript toolchain.
const PREBUILT: &[&str] = &[
    "sample_js",
    "sample_ts",
    "sample_settings_js",
    "sample_settings_ts",
    "sample_operations_js",
    "sample_operations_ts",
    "sample_applications_js",
    "sample_applications_ts",
    "sample_query_js",
    "sample_query_ts",
    "sample_search_js",
    "sample_search_ts",
    "sample_helper_js",
    "sample_helper_ts",
    "sample_files_js",
    "sample_files_ts",
    "sample_npm_js",
    "sample_background_js",
    "sample_background_ts",
];

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    let result = match task.as_deref() {
        Some("guests") => guests(),
        Some("js-guests") => js_guests(),
        Some("ci") => ci(),
        _ => Err("usage: cargo xtask <guests|js-guests|ci>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

fn run(command: &mut Command) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|error| format!("failed to start {command:?}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{command:?} failed with {status}"))
    }
}

/// Builds each guest workspace and copies its components to `target/guests`.
fn guests() -> Result<(), String> {
    let root = root();
    let out = root.join("target/guests");
    std::fs::create_dir_all(&out).map_err(|error| error.to_string())?;
    // (workspace directory, component file names)
    let workspaces: [(&str, &[&str]); 2] = [
        (
            "guests",
            &[
                "sample_rust",
                "sample_settings",
                "calculator",
                "applications",
                "quicklinks",
                "files",
                "sample_operations",
                "sample_dependencies",
                "sample_query",
                "sample_search",
                "sample_helper",
                "sample_background",
                "faulty",
                "operations_fixture",
                "old_api",
                "mismatched_api",
                "failing_start",
                "refusing_view",
            ],
        ),
        ("guests/fixtures/mixed-p2", &["mixed_p2"]),
    ];
    for (dir, components) in workspaces {
        let dir = root.join(dir);
        run(cargo().current_dir(&dir).args([
            "build",
            "--locked",
            "--release",
            "--target",
            GUEST_TARGET,
        ]))?;
        for name in components {
            let built = dir.join(format!("target/{GUEST_TARGET}/release/{name}.wasm"));
            let dest = out.join(format!("{name}.wasm"));
            std::fs::copy(&built, &dest)
                .map_err(|error| format!("copy {} failed: {error}", built.display()))?;
        }
    }
    for name in PREBUILT {
        let prebuilt = root.join(format!("guests/prebuilt/{name}.wasm"));
        std::fs::copy(&prebuilt, out.join(format!("{name}.wasm")))
            .map_err(|error| format!("copy {} failed: {error}", prebuilt.display()))?;
    }
    // Ready-to-run sample packages: each manifest in guests/packages with the
    // component it names.
    for (package, component) in SAMPLE_PACKAGES {
        let dest = out.join("packages").join(package);
        std::fs::create_dir_all(&dest).map_err(|error| error.to_string())?;
        let copies = [
            (
                root.join(format!("guests/packages/{package}/pane.json")),
                dest.join("pane.json"),
            ),
            (
                out.join(format!("{component}.wasm")),
                dest.join(format!("{component}.wasm")),
            ),
        ];
        for (from, to) in copies {
            std::fs::copy(&from, &to)
                .map_err(|error| format!("copy {} failed: {error}", from.display()))?;
        }
    }
    echo_helper(&root, &out)?;
    npm_sample(&root, &out)?;
    println!("guests built into {}", out.display());
    Ok(())
}

/// Builds the helper samples' native helper, `pane-echo`, for the system
/// this runs on, and puts it in each helper sample package (Rust,
/// JavaScript, TypeScript) as that target's file, as its `pane.json` names
/// it. CI runs this natively on each of its three systems, so each runs a
/// real helper built for it. An author ships one build per target;
/// Pane runs the one for its own system and compiles nothing.
fn echo_helper(root: &Path, out: &Path) -> Result<(), String> {
    let dir = root.join("guests/helpers/echo");
    run(cargo()
        .current_dir(&dir)
        .args(["build", "--locked", "--release"]))?;
    let target = pane_target::Target::current()
        .ok_or("Pane names no helper target for this system and processor")?;
    let file = format!("pane-echo{}", target.exe_suffix());
    let target = target.id();
    let built = dir.join("target/release").join(&file);
    for package in ["sample-helper", "sample-helper-js", "sample-helper-ts"] {
        let dest = out
            .join("packages")
            .join(package)
            .join("helpers")
            .join(&target);
        std::fs::create_dir_all(&dest).map_err(|error| error.to_string())?;
        std::fs::copy(&built, dest.join(&file))
            .map_err(|error| format!("copy {} failed: {error}", built.display()))?;
    }
    Ok(())
}

/// Assembles the npm sample (`guests/npm/greeter`: its `package.json`, its
/// `pane.json` and the components it names) into `target/guests/npm/greeter/`
/// and packs it into `target/guests/npm/pane-samples-greeter-0.1.0.tgz`, the
/// tarball `npm pack` makes of that folder: every file under `package/`, in a
/// gzipped tar. The tarball is the same on every system (fixed times, owners
/// and modes, files in name order), so the local registry of the tests and
/// smokes serves one integrity everywhere. It is never published.
fn npm_sample(root: &Path, out: &Path) -> Result<(), String> {
    let source = root.join("guests/npm/greeter");
    let dest = out.join("npm/greeter");
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest).map_err(|error| error.to_string())?;
    let mut files = vec!["package.json".to_owned(), "pane.json".to_owned()];
    let manifest = std::fs::read_to_string(source.join("package.json"))
        .map_err(|error| format!("read the npm sample's package.json failed: {error}"))?;
    // Its one component, the prebuilt `sample_npm_js`.
    let component = "sample_npm_js.wasm";
    if !manifest.contains(component) {
        return Err(format!(
            "the npm sample's package.json does not list {component}"
        ));
    }
    files.push(component.to_owned());
    for file in &files {
        let from = match file.ends_with(".wasm") {
            true => out.join(file),
            false => source.join(file),
        };
        std::fs::copy(&from, dest.join(file))
            .map_err(|error| format!("copy {} failed: {error}", from.display()))?;
    }
    files.sort();
    // npm's own fixed time for packed files, 1985-10-26T08:15:00Z.
    const NPM_MTIME: u64 = 499_162_500;
    let mut tar = tar::Builder::new(Vec::new());
    for file in &files {
        let contents = std::fs::read(dest.join(file)).map_err(|error| error.to_string())?;
        let mut header = tar::Header::new_ustar();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(NPM_MTIME);
        header.set_uid(0);
        header.set_gid(0);
        header.set_entry_type(tar::EntryType::Regular);
        tar.append_data(&mut header, format!("package/{file}"), contents.as_slice())
            .map_err(|error| error.to_string())?;
    }
    let tar = tar.into_inner().map_err(|error| error.to_string())?;
    let mut gz = flate2::GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), flate2::Compression::best());
    std::io::Write::write_all(&mut gz, &tar).map_err(|error| error.to_string())?;
    let tgz = gz.finish().map_err(|error| error.to_string())?;
    let packed = out.join("npm/pane-samples-greeter-0.1.0.tgz");
    std::fs::write(&packed, tgz)
        .map_err(|error| format!("write {} failed: {error}", packed.display()))?;
    Ok(())
}

/// (package folder in `guests/packages`, component) of each sample package,
/// and of the default extensions (the calculator, applications and
/// quicklinks).
const SAMPLE_PACKAGES: [(&str, &str); 31] = [
    ("sample-rust", "sample_rust"),
    ("sample-settings", "sample_settings"),
    ("sample-js", "sample_js"),
    ("sample-ts", "sample_ts"),
    ("sample-settings-js", "sample_settings_js"),
    ("sample-settings-ts", "sample_settings_ts"),
    ("calculator", "calculator"),
    ("applications", "applications"),
    ("quicklinks", "quicklinks"),
    ("files", "files"),
    ("sample-operations", "sample_operations"),
    ("sample-operations-js", "sample_operations_js"),
    ("sample-operations-ts", "sample_operations_ts"),
    ("sample-dependencies", "sample_dependencies"),
    ("sample-dependencies-npm", "sample_dependencies"),
    ("sample-applications-js", "sample_applications_js"),
    ("sample-applications-ts", "sample_applications_ts"),
    ("sample-query", "sample_query"),
    ("sample-query-js", "sample_query_js"),
    ("sample-query-ts", "sample_query_ts"),
    ("sample-search", "sample_search"),
    ("sample-search-js", "sample_search_js"),
    ("sample-search-ts", "sample_search_ts"),
    ("sample-helper", "sample_helper"),
    ("sample-helper-js", "sample_helper_js"),
    ("sample-helper-ts", "sample_helper_ts"),
    ("sample-files-js", "sample_files_js"),
    ("sample-files-ts", "sample_files_ts"),
    ("sample-background", "sample_background"),
    ("sample-background-js", "sample_background_js"),
    ("sample-background-ts", "sample_background_ts"),
];

/// Rebuilds `guests/prebuilt/` from the JS/TS sample sources, then refreshes
/// `target/guests/`. `PYTHON` names the interpreter if the default is absent.
fn js_guests() -> Result<(), String> {
    run(&mut pane_js("samples"))?;
    guests()
}

/// Runs a `tools/componentize-js/pane_js.py` subcommand with `PYTHON`, or the
/// platform's usual interpreter name.
fn pane_js(subcommand: &str) -> Command {
    let python = std::env::var_os("PYTHON")
        .unwrap_or_else(|| if cfg!(windows) { "python" } else { "python3" }.into());
    let mut command = Command::new(python);
    command
        .current_dir(root())
        .args(["tools/componentize-js/pane_js.py", subcommand]);
    command
}

fn ci() -> Result<(), String> {
    guests()?;
    // The prebuilt JS/TS samples must match their sources and pins.
    run(&mut pane_js("check"))?;
    let root = root();
    run(cargo().current_dir(&root).args(["fmt", "--all", "--check"]))?;
    for dir in [
        "guests",
        "guests/fixtures/mixed-p2",
        "guests/helpers/echo",
        "guests/hello-rust",
    ] {
        run(cargo()
            .current_dir(root.join(dir))
            .args(["fmt", "--all", "--check"]))?;
    }
    let clippy = [
        "clippy",
        "--locked",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ];
    run(cargo().current_dir(&root).args(clippy))?;
    // The helper sample's native helper, an ordinary program of its own.
    run(cargo()
        .current_dir(root.join("guests/helpers/echo"))
        .args(clippy))?;
    run(cargo()
        .current_dir(&root)
        .args(["test", "--locked", "--workspace"]))
}
