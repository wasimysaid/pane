#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{App, Bounds, TitlebarOptions, WindowBounds, WindowOptions, prelude::*, px, size};
use pane::LauncherWindow;
use pane_core::develop::Toolchains;
use pane_core::{Launcher, Runtime};

/// What `--install` asks to show for installation.
enum ToPreview {
    Folder(PathBuf),
    /// `npm:<name>` or `npm:<name>@<version>`.
    Npm(String),
}

/// `pane [--install <folder> | --install npm:<package>[@<version>]]`:
/// `--install` opens with the package in `<folder>` shown for installation,
/// as if chosen with the folder picker, or the npm package, as if named in
/// "Install extension from npm…".
fn package_to_preview() -> Option<ToPreview> {
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--install" {
            let source = args.next()?;
            return Some(match source.to_str().and_then(|s| s.strip_prefix("npm:")) {
                Some(spec) => ToPreview::Npm(spec.to_owned()),
                None => ToPreview::Folder(PathBuf::from(source)),
            });
        }
    }
    None
}

fn main() {
    let preview = package_to_preview();
    gpui_platform::application().run(move |cx: &mut App| {
        pane::bind_keys(cx);
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        let runtime = match pane::cache_dir() {
            Some(dir) => Runtime::start_with_cache(dir),
            None => Runtime::start(),
        };
        // Quitting ends the native helpers still running, which would
        // otherwise outlive Pane.
        if let Ok(runtime) = &runtime {
            // The native smokes crash the runtime on purpose, to check that
            // Pane recovers (#17); nothing else sets this, and a release
            // build has no such hook.
            #[cfg(debug_assertions)]
            if let Some(file) = std::env::var_os("PANE_TEST_RUNTIME_FAULTS") {
                runtime.watch_fault_file(PathBuf::from(file));
            }
            let runtime = runtime.clone();
            cx.on_app_quit(move |_| {
                runtime.stop_helpers();
                async {}
            })
            .detach();
        }
        let launcher = match pane::data_dir() {
            Some(dir) => {
                Launcher::with_packages(runtime, pane::sample_commands(), dir.join("extensions"))
            }
            None => Launcher::new(runtime, pane::sample_commands()),
        }
        .with_link_opener(Arc::new(pane::SystemLinks));
        // Development builds can download npm packages from a registry on
        // this computer instead (the tests' and smokes' own); release builds
        // always use registry.npmjs.org.
        #[cfg(debug_assertions)]
        let launcher = match pane_core::npm::Registry::from_dev_env() {
            Some(Ok(registry)) => launcher.with_npm_registry(registry),
            Some(Err(why)) => {
                eprintln!("PANE_NPM_REGISTRY: {why}");
                launcher.show_error(format!("PANE_NPM_REGISTRY: {why}"));
                launcher
            }
            None => launcher,
        };
        // Global hotkeys: the system's adapter is made on the main thread,
        // whose run loop receives the presses on macOS.
        let (press_sender, mut presses) = pane_core::hotkeys::channel();
        let launcher = launcher.with_hotkeys(pane_core::hotkeys::native(press_sender));
        // Clipboard history: Pane watches the clipboard only while an
        // enabled package keeps history the user turned on.
        let launcher = launcher.with_clipboard(pane_core::clipboard::native());
        // Development mode builds with the author's tools; a JavaScript or
        // TypeScript package with this checkout's build unless
        // PANE_COMPONENTIZE_JS names another.
        let (change_sender, changes) = pane_core::changes::channel();
        let default_js = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/componentize-js/pane_js.py");
        let toolchains = Toolchains::from_env(Some(default_js));
        let launcher = launcher.with_development(Arc::new(toolchains), change_sender);
        // Scheduled tasks the user turned on run in the background, on the
        // system's clock; after development, so the window redraws for them.
        let launcher = launcher.with_background(Arc::new(pane_core::clock::SystemClock));
        let bounds = Bounds::centered(None, size(px(640.), px(420.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Pane".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let window = cx
            .open_window(options, |window, cx| {
                cx.new(|cx| {
                    let mut launcher = LauncherWindow::new(launcher, window, cx);
                    launcher.follow_changes(changes, window, cx);
                    match &preview {
                        Some(ToPreview::Folder(folder)) => {
                            launcher.preview_package(folder, window, cx)
                        }
                        Some(ToPreview::Npm(spec)) => launcher.preview_npm(spec, window, cx),
                        None => {}
                    }
                    launcher
                })
            })
            .expect("failed to open the Pane window");
        // A hotkey pressed in any application opens its command here.
        cx.spawn(async move |cx| {
            while let Some(shortcut) = presses.next().await {
                let shown = window.update(cx, |launcher, window, cx| {
                    launcher.hotkey_pressed(&shortcut, window, cx)
                });
                if shown.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.activate(true);
    });
}
