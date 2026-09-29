#!/usr/bin/env bash
# Native GUI smoke on macOS: launches Pane, drives it with real key events
# (System Events via osascript, which needs the Accessibility permission the
# GitHub macOS runners grant) and captures and checks the screen.
# Requires Python 3 with Pillow for the screenshot checks. Pane keeps
# installed packages in <output-dir>/data, not the user's data folder.
# Usage: scripts/smoke-macos.sh <output-dir> [pane-binary]
set -euo pipefail
out=${1:-smoke}
pane=${2:-target/debug/pane}
mkdir -p "$out"
rm -rf "$out/data"
export PANE_DATA_DIR=$out/data
{ sw_vers; uname -m; } >"$out/system.txt"   # the tested OS version and architecture

pid=
npm_registry_pid=
trap '[ -n "$pid" ] && kill "$pid" 2>/dev/null; [ -n "$npm_registry_pid" ] && kill "$npm_registry_pid" 2>/dev/null || true' EXIT

capture() { screencapture -x "$out/$1"; }
check() { python3 "$(dirname "$0")/check_screenshot.py" "$out/$1" "$2" ${3:+"$3"}; }
# Prints "x y": where the screenshot shows the given color.
locate() { python3 "$(dirname "$0")/check_screenshot.py" --locate "$out/$1" "$2"; }
# Clicks the primary button at x y in the pixels of screenshot $3: Quartz
# events through ctypes, in points, which are half the pixels on Retina.
click_at() {
  python3 - "$1" "$2" "$out/$3" <<'PY'
import ctypes, sys
from PIL import Image
class CGPoint(ctypes.Structure): _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double)]
class CGRect(ctypes.Structure): _fields_ = [("origin", CGPoint), ("size", CGPoint)]
cg = ctypes.cdll.LoadLibrary("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics")
cg.CGMainDisplayID.restype = ctypes.c_uint32
cg.CGDisplayBounds.restype, cg.CGDisplayBounds.argtypes = CGRect, [ctypes.c_uint32]
cg.CGEventCreateMouseEvent.restype = ctypes.c_void_p
cg.CGEventCreateMouseEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint32, CGPoint, ctypes.c_uint32]
cg.CGEventPost.argtypes = [ctypes.c_uint32, ctypes.c_void_p]
scale = cg.CGDisplayBounds(cg.CGMainDisplayID()).size.x / Image.open(sys.argv[3]).width
at = CGPoint(int(sys.argv[1]) * scale, int(sys.argv[2]) * scale)
for event in (1, 2):  # left mouse down, left mouse up
    cg.CGEventPost(0, cg.CGEventCreateMouseEvent(None, event, at, 0))
PY
}
key() {  # macOS virtual key codes: 36 Return, 125 Down, 126 Up, 124 Right, 53 Escape, 48 Tab
  osascript -e "tell application \"System Events\" to key code $1"
}
type_text() { osascript -e "tell application \"System Events\" to keystroke \"$1\""; }
command_key() { osascript -e "tell application \"System Events\" to keystroke \"$1\" using command down"; }

# Brings the running Pane to the front, so that key events reach it.
focus_pane() {
  osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $pid) to true"
  sleep 1
}

# Starts Pane with the given arguments and brings it to the front.
start_pane() {
  "$pane" "$@" 2>>"$out/stderr.log" &
  pid=$!
  sleep 8
  focus_pane
}

stop_pane() {
  kill -0 "$pid" || { echo "Pane exited during the smoke"; exit 1; }
  kill "$pid"
  wait "$pid" 2>/dev/null || true
  pid=
}

start_pane
capture 1-root.png
check 1-root.png 8a96a3   # the hint line: text renders

# Open each sample command (Rust, JavaScript, TypeScript) and run an item.
for index in 0 1 2; do
  for ((i = 0; i < index; i++)); do key 125; done   # not seq: BSD "seq 0" prints 1 0
  key 36; sleep 3
  capture "$((index + 2))-command-$index.png"
  key 125; key 36; sleep 2
  capture "$((index + 2))-result-$index.png"
  check "$((index + 2))-result-$index.png" 9fd8a8   # the guest's answer
  key 53; sleep 1
done
capture 5-back-to-root.png
# Each command must have answered from its own guest, not the same view twice.
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{2,3,4}-result-*.png

# The Rust command's form (its fifth item): submitting it empty is rejected
# and focus returns to the name, so typing there and choosing a greeting with
# Tab and Down makes the guest answer.
key 36; sleep 3
for _ in 1 2 3 4; do key 125; done
key 36; sleep 1
capture 6-form.png
key 36; sleep 2
capture 7-form-error.png
check 7-form-error.png f08c8c   # the rejected field's message
type_text Ada
key 48; key 125; key 36; sleep 2
capture 8-form-result.png
check 8-form-result.png 9fd8a8   # the guest's answer
key 53; key 53; sleep 1
stop_pane

# Install the assembled Rust sample package (the folder the picker would
# return), then run its command. Root lists the three samples, the installed
# command, then the install and Manage extensions… rows.
start_pane --install target/guests/packages/sample-rust
capture 9-package.png
check 9-package.png aab4c0   # the package's identity and compatibility lines
key 36; sleep 2
capture 10-installed.png
check 10-installed.png 9fd8a8   # "Installed Rust sample"
key 36; sleep 3
key 36; sleep 2
capture 11-installed-result.png
check 11-installed-result.png 9fd8a8   # the installed guest's answer
stop_pane

# The installed command is still listed after a restart.
start_pane
capture 12-restarted.png
check 12-restarted.png 8a96a3
[ -f "$out/data/extensions/installed.json" ] || { echo "no install record"; exit 1; }
focus_pane

# The Rust command's seventh item is declared for Windows only, its eighth
# for macOS and Linux only. Here the first is explained without running and
# the second runs.
key 36; sleep 3
for _ in 1 2 3 4 5 6; do key 125; done
key 36; sleep 2
capture 13-windows-only.png
check 13-windows-only.png d6a36a   # the row's reason
check 13-windows-only.png f08c8c   # macOS: the reason as the error
key 125; key 36; sleep 2
capture 14-not-windows.png
check 14-not-windows.png 9fd8a8    # macOS: the guest's answer
key 53; sleep 1
stop_pane

# A package that supports only the other two systems has nothing for this
# one: it is explained instead of offered for installation.
mkdir -p "$out/elsewhere"
cp target/guests/sample_rust.wasm "$out/elsewhere/"
cat >"$out/elsewhere/pane.json" <<'JSON'
{
  "manifestVersion": 1,
  "title": "Elsewhere",
  "apiVersion": "0.1",
  "platforms": ["windows", "linux"],
  "commands": [{ "id": "sample", "title": "Elsewhere sample", "component": "sample_rust.wasm" }]
}
JSON
start_pane --install "$out/elsewhere"
capture 15-no-compatible-package.png
check 15-no-compatible-package.png f08c8c   # "Not available on macOS: ..."
stop_pane

# Install the settings sample, save a choice with it, then disable it in
# Manage extensions. Root lists the three samples, Rust sample, Greeting, the
# install row, then Manage extensions… last; the extension list holds Rust
# sample, then Settings sample.
start_pane --install target/guests/packages/sample-settings
key 36; sleep 2   # Install; Greeting is selected
key 36; sleep 3   # open Greeting
key 36; sleep 2   # "Use a formal greeting"
capture 16-setting-saved.png
check 16-setting-saved.png 9fd8a8   # "Saved the formal greeting"
key 53; sleep 1
for ((i = 0; i < 10; i++)); do key 125; done   # the last row
key 36; sleep 1
key 125; key 36; sleep 2
capture 17-disabled.png
check 17-disabled.png 9fd8a8   # "Disabled Settings sample"
stop_pane
grep -q '"disabled": true' "$out/data/extensions/installed.json" || { echo "disabled state not recorded"; exit 1; }
grep -q '"greeting-style": "formal"' "$out/data/extensions/settings.json" || { echo "setting not saved"; exit 1; }

# After a restart Greeting is no longer in root search: root looks exactly as
# it did before the settings sample was installed. Enabling the package again
# brings it back with its setting: "Greet me" answers in the saved formal
# style, where without a saved style it reports an error.
start_pane
capture 18-restarted-disabled.png
check 18-restarted-disabled.png 8a96a3
python3 "$(dirname "$0")/check_screenshot.py" --same "$out/12-restarted.png" "$out/18-restarted-disabled.png"
for ((i = 0; i < 10; i++)); do key 125; done
key 36; sleep 1
key 125; key 36; sleep 2
capture 19-enabled.png
check 19-enabled.png 9fd8a8   # "Enabled Settings sample"
key 53; sleep 1
for ((i = 0; i < 4; i++)); do key 125; done   # Greeting
key 36; sleep 3
key 125; key 125; key 36; sleep 2   # "Greet me"
capture 20-greeted.png
check 20-greeted.png 9fd8a8   # "Good day to you"
stop_pane

# Restarted, root lists Greeting again, after Rust sample.
start_pane
focus_pane

# The Rust command's color picker (its sixth item), which the guest draws:
# Right chooses purple, and a click on the dark green swatch chooses it. The
# chosen color fills its swatch and the preview, far more pixels than any
# other swatch covers.
key 36; sleep 3
for _ in 1 2 3 4 5; do key 125; done
key 36; sleep 2
capture 21-color.png
check 21-color.png 1e88e5 3000   # blue, chosen when the view opens
key 124; sleep 1
capture 22-color-key.png
check 22-color-key.png 8e24aa 3000   # purple
read -r x y < <(locate 22-color-key.png 1b5e20)
click_at "$x" "$y" 22-color-key.png; sleep 1
capture 23-color-click.png
check 23-color-click.png 1b5e20 3000   # dark green
key 53; key 53; sleep 1
stop_pane

# Root search: typing narrows root to the matching commands and Enter opens
# the best match. "typescr" matches only TypeScript sample, whose "Wait
# briefly" answers exactly as in step 4. A query that matches nothing shows
# no results, and Enter then opens nothing.
start_pane
type_text typescr; sleep 1
capture 24-search.png
key 36; sleep 3
key 125; key 36; sleep 2
capture 25-search-result.png
check 25-search-result.png 9fd8a8   # the TypeScript guest's answer
python3 "$(dirname "$0")/check_screenshot.py" --same "$out/4-result-2.png" "$out/25-search-result.png"
key 53; sleep 1
type_text zzz; sleep 1
key 36; sleep 1
capture 26-no-results.png
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{1-root,24-search,25-search-result,26-no-results}.png
stop_pane

# The calculator, a default extension: an expression typed into root search
# lists its answer first, selected, and Enter copies it. Pasting the copy
# over the query and typing on shows exactly the screen typing the whole
# expression shows, so the clipboard held the answer.
start_pane --install target/guests/packages/calculator
key 36; sleep 2   # Install
type_text '6*7'; sleep 2
capture 27-answer.png
check 27-answer.png 364355 3000   # the selected answer row
key 36; sleep 1
capture 28-copied.png   # "Copied 42 to the clipboard"
command_key a; type_text '42+1'; sleep 2
capture 29-typed.png
command_key a; command_key v; type_text '+1'; sleep 2
capture 30-pasted.png
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{27-answer,28-copied,29-typed}.png
python3 "$(dirname "$0")/check_screenshot.py" --same "$out/29-typed.png" "$out/30-pasted.png"
stop_pane

# Operations: install the JavaScript operations sample, then the Rust one,
# whose command (Call from Rust, selected once installed) opens its form,
# takes the JavaScript package's identity (local: and the folder's resolved
# path) and a name, and calls that package's greet operation: "Hello, Rust,
# from JavaScript" comes from the other package's guest, started for the call.
start_pane --install target/guests/packages/sample-operations-js
key 36; sleep 2   # Install
capture 31-operations-target.png
check 31-operations-target.png 9fd8a8   # "Installed JavaScript operations sample"
stop_pane
start_pane --install target/guests/packages/sample-operations
key 36; sleep 2   # Install; Call from Rust is selected
key 36; sleep 3   # open Call from Rust
key 36; sleep 2   # "Greet through another extension": its form
type_text "local:$(cd target/guests/packages/sample-operations-js && pwd -P)"
key 48; type_text Rust
key 36; sleep 5   # Greet
capture 32-operation-answer.png
check 32-operation-answer.png 9fd8a8   # the JavaScript guest's answer
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{31-operations-target,32-operation-answer}.png
stop_pane

# Reload a development package while Pane stays open. Its command starts as
# the Rust sample; a new build of it is the JavaScript sample. Root lists the
# three samples, Rust sample, Greeting, Calculator, Call from JavaScript, Call
# from Rust, Dev sample (the ninth row), the install row, then Manage
# extensions… last; the extension list holds the six packages (Dev is the
# sixth), then their six Reload rows (Reload Dev is the twelfth).
mkdir -p "$out/dev"
cp target/guests/sample_rust.wasm "$out/dev/command.wasm"
cat >"$out/dev/pane.json" <<'JSON'
{
  "manifestVersion": 1,
  "title": "Dev",
  "apiVersion": "0.1",
  "commands": [{ "id": "sample", "title": "Dev sample", "component": "command.wasm" }]
}
JSON
start_pane --install "$out/dev"
key 36; sleep 2   # Install; Dev sample is selected
key 36; sleep 3
key 36; sleep 2   # "Say hello"
capture 33-dev-before.png
check 33-dev-before.png 9fd8a8   # "Hello from the Rust guest"
key 53; sleep 1
cp target/guests/sample_js.wasm "$out/dev/command.wasm"
for ((i = 0; i < 12; i++)); do key 125; done   # the last row
key 36; sleep 1
for ((i = 0; i < 11; i++)); do key 125; done   # Reload Dev
key 36; sleep 3
capture 34-reloaded.png
check 34-reloaded.png 9fd8a8   # "Reloaded Dev"
key 53; sleep 1
for ((i = 0; i < 8; i++)); do key 125; done   # Dev sample
key 36; sleep 3
key 36; sleep 2   # "Say hello"
capture 35-dev-after.png
check 35-dev-after.png 9fd8a8   # "Hello from the JavaScript guest"
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out/33-dev-before.png" "$out/35-dev-after.png"
key 53; sleep 1

# A build that fails the install checks (here its component is missing) is
# not reloaded: the working code keeps running, exactly as before.
rm "$out/dev/command.wasm"
for ((i = 0; i < 12; i++)); do key 125; done
key 36; sleep 1
for ((i = 0; i < 11; i++)); do key 125; done
key 36; sleep 2
capture 36-not-reloaded.png
check 36-not-reloaded.png f08c8c   # "Dev was not reloaded: ..."
key 53; sleep 1
for ((i = 0; i < 8; i++)); do key 125; done
key 36; sleep 3
key 36; sleep 2
capture 37-still-running.png
python3 "$(dirname "$0")/check_screenshot.py" --same "$out/35-dev-after.png" "$out/37-still-running.png"
key 53; sleep 1

# A build whose start fails is reported with Retry, after Reload Dev; this
# one saves a setting and fails its first start only, so Retry starts it.
cp target/guests/failing_start.wasm "$out/dev/command.wasm"
for ((i = 0; i < 12; i++)); do key 125; done
key 36; sleep 1
for ((i = 0; i < 11; i++)); do key 125; done
key 36; sleep 3
capture 38-start-failed.png
check 38-start-failed.png f08c8c   # "Reloaded Dev, but it failed to start; ..."
key 125; key 36; sleep 3   # Retry starting Dev
capture 39-retried.png
check 39-retried.png 9fd8a8   # "Started Dev"
stop_pane
grep -q '"start-attempted": "yes"' "$out/data/extensions/settings.json" || { echo "the failed start's setting was not kept"; exit 1; }

# The settings sample keeps one value of each kind of data: its formal style
# (settings) and "Good day to you" (cache) are saved above; its fourth and
# fifth items save a note (content) and sign in (a local credential), and its
# sixth shows all four.
start_pane
for ((i = 0; i < 4; i++)); do key 125; done   # Greeting
key 36; sleep 3
for ((i = 0; i < 3; i++)); do key 125; done
key 36; sleep 2   # "Save a note"
key 125; key 36; sleep 2   # "Sign in"
key 125; key 36; sleep 2   # "Show what Pane keeps"
capture 40-kept.png
check 40-kept.png 9fd8a8   # every value, the cached greeting included
key 53; sleep 1
stop_pane
grep -q '"note": "Water the plants"' "$out/data/extensions/content.json" || { echo "note not saved"; exit 1; }
grep -q '"token": "sample-token"' "$out/data/extensions/credentials.json" || { echo "credential not saved"; exit 1; }
grep -q '"last-greeting": "Good day to you"' "$out/data/extensions/cache.json" || { echo "greeting not cached"; exit 1; }

# Clear the settings sample's cache in Manage extensions: its row follows the
# six package rows, their six Reload rows and "Clear cache of Rust sample". Pane asks first, then deletes only the cached
# greeting, without running the extension.
start_pane
for ((i = 0; i < 12; i++)); do key 125; done   # the last row
key 36; sleep 1
for ((i = 0; i < 13; i++)); do key 125; done
key 36; sleep 1   # "Clear cache of Settings sample"
capture 41-confirm-clear-cache.png
check 41-confirm-clear-cache.png aab4c0   # what is deleted and what is kept
key 36; sleep 2   # "Clear cache"
capture 42-cache-cleared.png
check 42-cache-cleared.png 9fd8a8   # "Cleared the cache of Settings sample"
key 53; sleep 1
for ((i = 0; i < 4; i++)); do key 125; done   # Greeting
key 36; sleep 3
for ((i = 0; i < 5; i++)); do key 125; done
key 36; sleep 2   # "Show what Pane keeps"
capture 43-kept-after-clear.png
check 43-kept-after-clear.png 9fd8a8   # "... Cached greeting: none"
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out/40-kept.png" "$out/43-kept-after-clear.png"
key 53; sleep 1
stop_pane
if grep -q 'Good day to you' "$out/data/extensions/cache.json"; then echo "cache not cleared"; exit 1; fi
grep -q '"greeting-style": "formal"' "$out/data/extensions/settings.json" || { echo "setting lost"; exit 1; }
grep -q '"note": "Water the plants"' "$out/data/extensions/content.json" || { echo "note lost"; exit 1; }
grep -q '"token": "sample-token"' "$out/data/extensions/credentials.json" || { echo "credential lost"; exit 1; }

# Applications, a default extension: an installed application is found by
# name in root search and Enter opens it. The application is a bundle the
# smoke adds in ~/Applications of a HOME of its own (for Pane only), whose
# program writes a marker file, so nothing else is started; Pane still
# searches the system's applications too.
apps=$(cd "$out" && pwd)/apps
rm -rf "$apps"
bundle="$apps/home/Applications/Pane Smoke App.app"
mkdir -p "$bundle/Contents/MacOS"
cat >"$bundle/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>smoke</string>
<key>CFBundleIdentifier</key><string>dev.pane.smoke-app</string>
<key>CFBundleName</key><string>Pane Smoke App</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
EOF
printf '#!/bin/sh\necho launched > "%s"\n' "$apps/launched" >"$bundle/Contents/MacOS/smoke"
chmod +x "$bundle/Contents/MacOS/smoke"
HOME=$apps/home "$pane" --install target/guests/packages/applications 2>>"$out/stderr.log" &
pid=$!
sleep 8
focus_pane
key 36; sleep 2   # Install
type_text 'pane smoke'; sleep 3
capture 44-application.png
check 44-application.png 364355 3000   # the selected application row
key 36; sleep 3
focus_pane
capture 45-opened.png
check 45-opened.png 9fd8a8   # "Opened Pane Smoke App"
for _ in $(seq 50); do [ -f "$apps/launched" ] && break; sleep 0.2; done
[ -f "$apps/launched" ] || { echo "the application did not run"; exit 1; }
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{44-application,45-opened}.png
stop_pane

# Quicklinks, a default extension: installed, its command's form saves a
# quicklink (Quicklinks is selected once installed, and "Create quicklink" is
# its first item). After a restart, typing part of its name lists it,
# selected. Enter would open the default browser, so this smoke stops there
# (the Linux smoke opens it through a recording handler).
start_pane --install target/guests/packages/quicklinks
key 36; sleep 2   # Install
key 36; sleep 3   # open Quicklinks
key 36; sleep 1   # Create quicklink
type_text 'Pane issues'
key 48
type_text 'https://example.com/pane-issues'
key 36; sleep 2
capture 46-quicklink-saved.png
check 46-quicklink-saved.png 9fd8a8   # "Saved quicklink “Pane issues”"
key 53; key 53; sleep 1
stop_pane
start_pane
type_text 'pane iss'; sleep 2
capture 47-quicklink-found.png
check 47-quicklink-found.png 364355 3000   # the selected quicklink row
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{46-quicklink-saved,47-quicklink-found}.png
stop_pane

# Uninstall the settings sample, keeping its saved data: its row follows the
# eight Clear cache rows. Pane asks first, showing its saved data, and the first
# choice keeps its settings and content while its copy and credential go.
# Installing the same folder again finds its formal style and note, signed out.
start_pane
for ((i = 0; i < 20; i++)); do key 125; done   # the last row
key 36; sleep 1
for ((i = 0; i < 25; i++)); do key 125; done
key 36; sleep 1   # "Uninstall Settings sample"
capture 49-confirm-uninstall.png
check 49-confirm-uninstall.png aab4c0   # what is removed and the saved data
key 36; sleep 2   # "Uninstall and keep saved data"
capture 50-uninstalled.png
check 50-uninstalled.png 9fd8a8   # "Uninstalled Settings sample; its settings and content are kept"
stop_pane
grep -q '"retained"' "$out/data/extensions/installed.json" || { echo "kept data not recorded"; exit 1; }
if grep -q 'sample-token' "$out/data/extensions/credentials.json"; then echo "credential not removed"; exit 1; fi
grep -q '"greeting-style": "formal"' "$out/data/extensions/settings.json" || { echo "setting not kept"; exit 1; }
grep -q '"note": "Water the plants"' "$out/data/extensions/content.json" || { echo "note not kept"; exit 1; }
start_pane --install target/guests/packages/sample-settings
key 36; sleep 2   # Install; Greeting is selected
key 36; sleep 3   # open Greeting
for ((i = 0; i < 5; i++)); do key 125; done
key 36; sleep 2   # "Show what Pane keeps"
capture 51-reinstalled.png
check 51-reinstalled.png 9fd8a8   # "Style: formal · Note: Water the plants · Signed in: no ..."
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out/43-kept-after-clear.png" "$out/51-reinstalled.png"
key 53; sleep 1
stop_pane
if grep -q '"retained"' "$out/data/extensions/installed.json"; then echo "retained record not dropped"; exit 1; fi

# Global hotkeys: in Manage extensions, the settings sample's command,
# Greeting, is given Control+Option+G by pressing it on its hotkey screen
# (its row follows the package's state, Reload, Clear cache and Uninstall
# rows). With Finder in front, pressing the hotkey brings Pane to the front
# with Greeting open, also after a restart; once the extension is disabled, pressing it
# does nothing. Carbon hot keys need no permission of Pane's own. A data
# folder of its own keeps the rows in a known order. (Screenshot 48 is the
# Linux smoke's opened quicklink.)
frontmost() { osascript -e 'tell application "System Events" to get unix id of first process whose frontmost is true'; }
unfocus_pane() {   # another application in front
  osascript -e 'tell application "Finder" to activate'; sleep 2
  [ "$(frontmost)" != "$pid" ] || { echo "Pane is still in front"; exit 1; }
}
press_hotkey() {
  osascript -e 'tell application "System Events" to keystroke "g" using {control down, option down}'
  sleep 3
}
check_pane_in_front() {
  [ "$(frontmost)" = "$pid" ] || { echo "the hotkey did not bring Pane to the front"; exit 1; }
}
export PANE_DATA_DIR=$out/hotkeys-data
rm -rf "$PANE_DATA_DIR"
start_pane --install target/guests/packages/sample-settings
key 36; sleep 2   # Install; Greeting is selected
for ((i = 0; i < 10; i++)); do key 125; done   # Manage extensions…
key 36; sleep 1
key 125; key 125; key 125; key 125; key 36; sleep 1   # "Hotkey for Greeting"
capture 52-hotkey-screen.png
check 52-hotkey-screen.png aab4c0   # "Press the keys that should open Greeting ..."
press_hotkey   # Pane is in front: this assigns it
capture 53-hotkey-assigned.png
check 53-hotkey-assigned.png 9fd8a8   # "Control+Option+G now opens Greeting"
key 53; sleep 1   # root search
unfocus_pane
capture 54-unfocused.png
press_hotkey
check_pane_in_front
capture 55-hotkey-opened.png
check 55-hotkey-opened.png 364355 3000   # Greeting's first item, selected
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{53-hotkey-assigned,55-hotkey-opened}.png
stop_pane
grep -q '"ctrl+alt+g"' "$PANE_DATA_DIR/extensions/hotkeys.json" || { echo "hotkey not recorded"; exit 1; }
start_pane
unfocus_pane
press_hotkey
check_pane_in_front
capture 56-hotkey-after-restart.png
check 56-hotkey-after-restart.png 364355 3000   # Greeting's first item, selected
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{53-hotkey-assigned,56-hotkey-after-restart}.png
key 53; sleep 1
for ((i = 0; i < 10; i++)); do key 125; done   # Manage extensions…
key 36; sleep 1
key 36; sleep 2   # disable Settings sample
key 53; sleep 1
capture 57-disabled.png   # root search
unfocus_pane
press_hotkey
[ "$(frontmost)" != "$pid" ] || { echo "the released hotkey still brought Pane to the front"; exit 1; }
focus_pane
capture 58-disabled-pressed.png   # still root search: nothing opened
python3 "$(dirname "$0")/check_screenshot.py" --same "$out"/{57-disabled,58-disabled-pressed}.png
stop_pane

# Pausing a broken extension: the settings sample's last item, Crash, crashes
# on purpose; the third crash within five minutes pauses the package and
# returns to root search, where Greeting stays listed with why it does not
# run. The pause holds after a restart. In Manage extensions, the package's
# "Why ... is paused" row (after its Reload and Retry rows) shows the
# details, whose only row, Retry, starts it again. A data folder of its own
# keeps the rows in a known order.
export PANE_DATA_DIR=$out/pausing-data
rm -rf "$PANE_DATA_DIR"
start_pane --install target/guests/packages/sample-settings
key 36; sleep 2   # Install; Greeting is selected
key 36; sleep 2   # open Greeting
for ((i = 0; i < 7; i++)); do key 125; done   # Crash
for ((i = 0; i < 3; i++)); do key 36; sleep 2; done
type_text greet; sleep 1   # Greeting and its reason at the top on any window height
capture 59-paused.png
check 59-paused.png f08c8c   # "Settings sample crashed 3 times within 5 minutes and is paused ..."
check 59-paused.png d6a36a   # Greeting: "Settings sample is paused after an error; ..."
stop_pane
grep -q '"paused"' "$PANE_DATA_DIR/extensions/installed.json" || { echo "pause not recorded"; exit 1; }
start_pane
type_text greet; sleep 1
capture 60-paused-after-restart.png
check 60-paused-after-restart.png d6a36a   # Greeting is still paused
key 53; sleep 1   # Escape clears the query
for ((i = 0; i < 10; i++)); do key 125; done   # Manage extensions…
key 36; sleep 1
key 125; key 125; key 125; key 36; sleep 1   # "Why Settings sample is paused"
capture 61-pause-details.png
check 61-pause-details.png aab4c0   # the details
key 36; sleep 2   # Retry Settings sample
capture 62-pause-retried.png
check 62-pause-retried.png 9fd8a8   # "Started Settings sample"
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{61-pause-details,62-pause-retried}.png
stop_pane
if grep -q '"paused"' "$PANE_DATA_DIR/extensions/installed.json"; then echo "pause not cleared"; exit 1; fi

# Delete retained data: with a data folder of its own, the settings sample
# saves a note and is uninstalled keeping it (its Uninstall row follows its
# state, Reload and Clear cache rows); its retained data, the extension list's
# last row, is deleted after confirming (Cancel is selected first, so Down
# then Return), without the extension. Installing the same folder again finds
# nothing. Steps that change Pane's files wait for the change instead of a
# fixed time.
export PANE_DATA_DIR=$out/retained-data
rm -rf "$PANE_DATA_DIR"
# Waits until file $1 contains text $2 ("present") or no longer does ("absent").
wait_for() {
  for _ in $(seq 100); do
    if grep -q "$2" "$1" 2>/dev/null; then [ "$3" = present ] && return; else [ "$3" = absent ] && return; fi
    sleep 0.1
  done
  echo "$1: $2 is not $3"; exit 1
}
registry=$PANE_DATA_DIR/extensions/installed.json
start_pane --install target/guests/packages/sample-settings
key 36   # Install; Greeting is selected
wait_for "$registry" sample-settings present; sleep 1
key 36; sleep 3   # open Greeting
for ((i = 0; i < 3; i++)); do key 125; done
key 36   # "Save a note"
wait_for "$PANE_DATA_DIR/extensions/content.json" '"note": "Water the plants"' present
key 53; sleep 1   # root search
for ((i = 0; i < 10; i++)); do key 125; done   # Manage extensions…
key 36; sleep 1
for ((i = 0; i < 3; i++)); do key 125; done
key 36; sleep 1   # "Uninstall Settings sample"
key 36   # "Uninstall and keep saved data"
wait_for "$registry" '"retained"' present; sleep 1
for ((i = 0; i < 40; i++)); do key 125; done
key 36; sleep 1   # "Delete retained data of Settings sample"
capture 63-confirm-delete-retained.png
check 63-confirm-delete-retained.png aab4c0   # what is kept and what is not touched
key 125; key 36   # "Delete retained data"
wait_for "$registry" '"retained"' absent; sleep 1
capture 64-retained-deleted.png
check 64-retained-deleted.png 9fd8a8   # "Deleted the retained data of Settings sample"
stop_pane
if grep -q 'Water the plants' "$PANE_DATA_DIR/extensions/content.json"; then echo "note not deleted"; exit 1; fi
start_pane --install target/guests/packages/sample-settings
key 36   # Install; Greeting is selected
wait_for "$registry" sample-settings present; sleep 1
key 36; sleep 3   # open Greeting
for ((i = 0; i < 5; i++)); do key 125; done
key 36; sleep 2   # "Show what Pane keeps"
capture 65-reinstalled-empty.png
check 65-reinstalled-empty.png 9fd8a8   # "Style: none · Note: none · Signed in: no ..."
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out/51-reinstalled.png" "$out/65-reinstalled-empty.png"
key 53; sleep 1
stop_pane

# Aliases and fallbacks: in Manage extensions, the query sample's command,
# Echo, is given the alias "ec" (its row follows the package's state, Reload,
# Clear cache, Uninstall and hotkey rows) and made a fallback (the next row).
# In root search, "ec hello" lists the row that sends "hello" to Echo,
# selected, and Enter shows Echo's answer; text nothing matches lists "No
# results" with Echo below it, not selected, until Down selects it and Enter
# sends the text. After a restart with the extension disabled, "ec hello"
# lists nothing: the same screen as a Pane with nothing installed. Data
# folders of their own keep the rows in a known order.
export PANE_DATA_DIR=$out/aliases-data
rm -rf "$PANE_DATA_DIR"
start_pane --install target/guests/packages/sample-query
key 36; sleep 2   # Install; Echo is selected
for ((i = 0; i < 10; i++)); do key 125; done   # Manage extensions…
key 36; sleep 1
for ((i = 0; i < 5; i++)); do key 125; done   # "Alias for Echo"
key 36; sleep 1
type_text ec
key 36; sleep 2
capture 66-alias-saved.png
check 66-alias-saved.png 9fd8a8   # "Typing “ec” now finds Echo"
key 125; key 36; sleep 2   # "Fallback: Echo"
capture 67-fallback-on.png
check 67-fallback-on.png 9fd8a8   # "Echo is now offered for any text typed in root search"
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{66-alias-saved,67-fallback-on}.png
key 53; sleep 1   # root search
type_text 'ec hello'; sleep 1
capture 68-alias-row.png
check 68-alias-row.png 364355 3000   # Echo, sending “hello”, selected
key 36; sleep 3
capture 69-alias-answer.png
check 69-alias-answer.png 9fd8a8   # "Echo heard “hello”"
key 53; sleep 1   # clears the query
type_text zqx; sleep 1
capture 70-fallback-listed.png   # "No results for “zqx”", then Echo, not selected
key 125; sleep 1
capture 71-fallback-chosen.png
check 71-fallback-chosen.png 364355 3000   # Echo, now selected
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{70-fallback-listed,71-fallback-chosen}.png
key 36; sleep 3
capture 72-fallback-answer.png
check 72-fallback-answer.png 9fd8a8   # "Echo heard “zqx”"
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{69-alias-answer,72-fallback-answer}.png
stop_pane
grep -q '"ec"' "$PANE_DATA_DIR/extensions/aliases.json" || { echo "alias not recorded"; exit 1; }
grep -q '#echo"' "$PANE_DATA_DIR/extensions/aliases.json" || { echo "fallback not recorded"; exit 1; }
start_pane
for ((i = 0; i < 10; i++)); do key 125; done   # Manage extensions…
key 36; sleep 1
key 36; sleep 2   # disable Query sample
key 53; sleep 1
type_text 'ec hello'; sleep 1
capture 73-alias-disabled.png   # "No results for “ec hello”"
stop_pane
grep -q '"disabled": true' "$PANE_DATA_DIR/extensions/installed.json" || { echo "not disabled"; exit 1; }
export PANE_DATA_DIR=$out/aliases-empty-data
rm -rf "$PANE_DATA_DIR"
start_pane
type_text 'ec hello'; sleep 1
capture 74-nothing-installed.png   # "No results for “ec hello”"
python3 "$(dirname "$0")/check_screenshot.py" --same "$out"/{73-alias-disabled,74-nothing-installed}.png
stop_pane

# Dependencies: the dependencies sample requires the JavaScript operations
# sample (from ../sample-operations-js) and can use the Rust one, which is
# optional. Its preview lists both; Install installs it with the JavaScript
# sample only, and its command (selected once installed) calls that
# package's greet operation by its dependency id: "Hello, Pane, from
# JavaScript" comes from the other package's guest. A data folder of its own
# starts with nothing installed.
export PANE_DATA_DIR=$out/dependencies-data
rm -rf "$PANE_DATA_DIR"
start_pane --install target/guests/packages/sample-dependencies
capture 75-dependencies-preview.png
check 75-dependencies-preview.png aab4c0   # "Requires: JavaScript operations sample, installed with it ..."
key 36; sleep 3   # Install; Greet through dependencies is selected
capture 76-dependencies-installed.png
check 76-dependencies-installed.png 9fd8a8   # "Installed Dependencies sample with JavaScript operations sample, which it requires"
key 36; sleep 3   # open Greet through dependencies
key 36; sleep 5   # Greet through the required greeter
capture 77-dependency-answer.png
check 77-dependency-answer.png 9fd8a8   # the JavaScript guest's answer
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{75-dependencies-preview,76-dependencies-installed,77-dependency-answer}.png
stop_pane
grep -q '"id": "greeter"' "$PANE_DATA_DIR/extensions/installed.json" || { echo "dependency not recorded"; exit 1; }
[ "$(grep -c '"dir"' "$PANE_DATA_DIR/extensions/installed.json")" = 2 ] || { echo "not exactly two packages installed"; exit 1; }

# Native helpers: the helper sample's command runs pane-echo, the file its
# package ships for this system (built by `cargo xtask guests`). Its first
# item shows the helper's answer, naming the system; its third races the
# helper against a one-second timer and cancels it. Its second has the
# helper wait ten seconds: disabling the package meanwhile (its row is the
# first in Manage extensions) ends the helper's process at once, and the
# note it saved before is kept. A data folder of its own keeps the rows in a
# known order; the helper runs from its managed copy there.
export PANE_DATA_DIR=$out/helper-data
rm -rf "$PANE_DATA_DIR"
# Pane's helper processes: pane-echo run from this data folder.
helpers_running() { pgrep -f "$PANE_DATA_DIR/extensions/packages/.*/pane-echo" >/dev/null; }
start_pane --install target/guests/packages/sample-helper
key 36; sleep 2   # Install; Helper sample is selected
key 36; sleep 2   # open Helper sample
key 36; sleep 2   # Echo through the helper
capture 90-helper-echoed.png
check 90-helper-echoed.png 9fd8a8   # 'Echoed "hello from Pane" on macOS arm64'
key 125; key 125; key 36; sleep 3   # Echo within a second
capture 91-helper-cancelled.png
check 91-helper-cancelled.png 9fd8a8   # "Stopped the helper after one second"
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{90-helper-echoed,91-helper-cancelled}.png
if helpers_running; then echo "a cancelled helper is still running"; exit 1; fi
key 126; key 36; sleep 2   # Up: Echo after waiting
helpers_running || { echo "the waiting helper is not running"; exit 1; }
capture 92-helper-waiting.png
key 53; sleep 1   # root search; the helper keeps running
for ((i = 0; i < 10; i++)); do key 125; done   # Manage extensions…
key 36; sleep 1
key 36; sleep 2   # disable Helper sample
capture 93-helper-disabled.png
check 93-helper-disabled.png 9fd8a8   # "Disabled Helper sample"
if helpers_running; then echo "the helper outlived its disabled package"; exit 1; fi
grep -q '"helper-wait": "started"' "$PANE_DATA_DIR/extensions/settings.json" || { echo "saved note lost"; exit 1; }
if grep -q '"helper-wait": "finished"' "$PANE_DATA_DIR/extensions/settings.json"; then echo "the stopped call finished"; exit 1; fi
stop_pane
if helpers_running; then echo "a helper outlived Pane"; exit 1; fi

# Quitting Pane while a helper runs ends it: with "Echo after waiting"
# running (the helper beats in pane-echo.alive in its folder of the managed
# copy), asking Pane to quit the way the Dock's Quit does (a quit Apple
# event, sent by NSRunningApplication's terminate) ends the helper first. A
# data folder of its own again.
export PANE_DATA_DIR=$out/helper-quit-data
rm -rf "$PANE_DATA_DIR"
start_pane --install target/guests/packages/sample-helper
key 36; sleep 2   # Install; Helper sample is selected
key 36; sleep 2   # open Helper sample
key 125; key 36; sleep 2   # Echo after waiting
helpers_running || { echo "the waiting helper is not running"; exit 1; }
capture 94-helper-before-quit.png
check 94-helper-before-quit.png d6c27a   # "Running…"
alive=$(find "$PANE_DATA_DIR/extensions/packages" -name pane-echo.alive | head -1)
[ -n "$alive" ] || { echo "the waiting helper does not beat"; exit 1; }
python3 - "$pid" <<'PY'
import ctypes, ctypes.util, sys
objc = ctypes.cdll.LoadLibrary(ctypes.util.find_library("objc"))
ctypes.cdll.LoadLibrary(ctypes.util.find_library("AppKit"))
objc.objc_getClass.restype = ctypes.c_void_p
objc.objc_getClass.argtypes = [ctypes.c_char_p]
objc.sel_registerName.restype = ctypes.c_void_p
objc.sel_registerName.argtypes = [ctypes.c_char_p]
address = ctypes.cast(objc.objc_msgSend, ctypes.c_void_p).value
by_pid = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int)(address)
terminate = ctypes.CFUNCTYPE(ctypes.c_bool, ctypes.c_void_p, ctypes.c_void_p)(address)
app = by_pid(objc.objc_getClass(b"NSRunningApplication"),
             objc.sel_registerName(b"runningApplicationWithProcessIdentifier:"), int(sys.argv[1]))
if not app:
    sys.exit("Pane is not a running application")
sys.exit(0 if terminate(app, objc.sel_registerName(b"terminate")) else "Pane did not take the quit request")
PY
for _ in $(seq 50); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; done
if kill -0 "$pid" 2>/dev/null; then echo "Pane did not quit when asked"; exit 1; fi
wait "$pid" 2>/dev/null || true
pid=
if helpers_running; then echo "a helper outlived Pane quitting"; exit 1; fi
beats=$(stat -f %z "$alive"); sleep 0.5
[ "$(stat -f %z "$alive")" = "$beats" ] || { echo "the helper still beats after Pane quit"; exit 1; }

# Development mode (#12, #13): a copy of each development sample
# (guests/hello-rust, hello-ts, hello-js) is built once, installed and
# developed from Manage extensions ("Develop <title>", its last row). Saving
# an edit of its greeting builds it with the documented command and reloads
# it while Pane keeps running; a save that does not build keeps the working
# code and shows the error; two saves in a row (the second while the first
# builds) end with the newer greeting; after "Stop developing", a save builds
# nothing. Each sample has a data folder of its own, so root lists the three
# built-in samples, then its command, the install and Manage extensions…
# rows. The JavaScript and TypeScript samples need the JS toolchain
# (guests/README.md) and are skipped without it.
set_greeting() {   # set_greeting <source file> <line replacing the greeting's>
  python3 - "$1" "$2" <<'PY'
import re, sys
path, line = sys.argv[1], sys.argv[2]
text = open(path, encoding="utf-8").read()
text = re.sub(r"^const GREETING.*$", lambda _: line, text, count=1, flags=re.M)
open(path, "w", encoding="utf-8").write(text)
PY
}
# Waits until Pane has reloaded a new build: the component built in the
# copy differs from $2 (the one before the save) and the managed copy is it.
wait_reloaded() {   # wait_reloaded <built component> <component before the save>
  for _ in $(seq 600); do
    managed=$(find "$PANE_DATA_DIR/extensions/packages" -name "$(basename "$1")" | head -1)
    if [ -n "$managed" ] && ! cmp -s "$1" "$2" && cmp -s "$1" "$managed"; then
      sleep 3; return
    fi
    sleep 0.5
  done
  echo "Pane did not reload $1"; exit 1
}
# Waits until Pane has reported one more build that did not build.
wait_failed() {   # wait_failed <failures before>
  for _ in $(seq 600); do
    [ "$(grep -c 'did not build' "$out/stderr.log")" -gt "$1" ] && { sleep 1; return; }
    sleep 0.5
  done
  echo "Pane did not report the failed build"; exit 1
}
say_hello() {   # from root: open the developed command, the 4th row, and run its item
  key 125; key 125; key 125; key 36; sleep 3
  key 36; sleep 2
}
develop_sample() {   # develop_sample <sample> <title> <component> <source> <first frame> <greeting line> <broken line>
  local sample=$1 title=$2 component=$3 source=$4 n=$5 greeting=$6 broken=$7
  export PANE_DATA_DIR=$out/develop-$sample-data
  rm -rf "$PANE_DATA_DIR"
  local copy=$out/develop-$sample
  rm -rf "$copy"
  mkdir -p "$copy"
  (cd "guests/$sample" && tar cf - --exclude=target --exclude=dist --exclude=node_modules .) | (cd "$copy" && tar xf -)
  if [ -f "$copy/Cargo.toml" ]; then
    cp rust-toolchain.toml "$copy/"
    python3 - "$copy/Cargo.toml" "$PWD/guests/pane-guest" <<'PY'
import sys
path, guest = sys.argv[1], sys.argv[2]
text = open(path, encoding="utf-8").read().replace('path = "../pane-guest"', "path = '%s'" % guest)
open(path, "w", encoding="utf-8").write(text)
PY
    (cd "$copy" && cargo build --release --target wasm32-wasip2 --quiet)
  else
    python3 tools/componentize-js/pane_js.py build "$copy" "$copy/$component" >/dev/null
  fi
  local built=$copy/$component before=$out/develop-$sample-before.wasm
  start_pane --install "$copy"
  key 36; sleep 2   # Install
  for ((i = 0; i < 10; i++)); do key 125; done   # Manage extensions…
  key 36; sleep 1
  for ((i = 0; i < 10; i++)); do key 125; done   # Develop <title>
  key 36; sleep 2
  capture "$n-$sample-develop-started.png"
  check "$n-$sample-develop-started.png" 9fd8a8   # "Developing <title>: each save in ..."
  key 53; sleep 1
  say_hello
  capture "$((n + 1))-$sample-greeting-before.png"
  check "$((n + 1))-$sample-greeting-before.png" 9fd8a8   # "Hello from ..."
  key 53; sleep 1

  # An edit, saved: built and reloaded.
  cp "$built" "$before"
  set_greeting "$copy/$source" "$(printf "$greeting" "Hello again")"
  wait_reloaded "$built" "$before"
  capture "$((n + 2))-$sample-rebuilt.png"
  check "$((n + 2))-$sample-rebuilt.png" 9fd8a8   # "Reloaded <title>"
  say_hello
  capture "$((n + 3))-$sample-greeting-after.png"
  check "$((n + 3))-$sample-greeting-after.png" 9fd8a8   # "Hello again"
  python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out/$((n + 1))-$sample-greeting-before.png" "$out/$((n + 3))-$sample-greeting-after.png"
  key 53; sleep 1

  # A save that does not build: the working code stays.
  local failures
  failures=$(grep -c 'did not build' "$out/stderr.log" || true)
  set_greeting "$copy/$source" "$broken"
  wait_failed "$failures"
  capture "$((n + 4))-$sample-build-failed.png"
  check "$((n + 4))-$sample-build-failed.png" f08c8c   # "<title> did not build: ..."
  say_hello
  capture "$((n + 5))-$sample-kept.png"
  check "$((n + 5))-$sample-kept.png" 9fd8a8   # still "Hello again"
  python3 "$(dirname "$0")/check_screenshot.py" --same "$out/$((n + 3))-$sample-greeting-after.png" "$out/$((n + 5))-$sample-kept.png"
  key 53; sleep 1

  # Two saves, the second while the first builds: the newer one is reloaded.
  cp "$built" "$before"
  set_greeting "$copy/$source" "$(printf "$greeting" "Hello once more")"
  sleep 0.5
  set_greeting "$copy/$source" "$(printf "$greeting" "Hello at last")"
  wait_reloaded "$built" "$before"
  capture "$((n + 6))-$sample-rebuilt-again.png"
  check "$((n + 6))-$sample-rebuilt-again.png" 9fd8a8   # "Reloaded <title>"
  say_hello
  capture "$((n + 7))-$sample-greeting-fixed.png"
  check "$((n + 7))-$sample-greeting-fixed.png" 9fd8a8   # "Hello at last"
  python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out/$((n + 3))-$sample-greeting-after.png" "$out/$((n + 7))-$sample-greeting-fixed.png"
  key 53; sleep 1

  # Stopped: a save builds nothing.
  for ((i = 0; i < 10; i++)); do key 125; done   # Manage extensions…
  key 36; sleep 1
  for ((i = 0; i < 10; i++)); do key 125; done   # Stop developing <title>
  key 36; sleep 2
  capture "$((n + 8))-$sample-stopped.png"
  check "$((n + 8))-$sample-stopped.png" 9fd8a8   # "Stopped developing <title>"
  cp "$built" "$before"
  set_greeting "$copy/$source" "$(printf "$greeting" "Hello unseen")"
  sleep 8
  cmp -s "$built" "$before" || { echo "$title was built after development stopped"; exit 1; }
  stop_pane
}
develop_sample hello-rust "Hello Rust" target/wasm32-wasip2/release/hello_rust.wasm src/lib.rs 110 \
  'const GREETING: &str = "%s from Rust";' 'const GREETING: &str = 42;'
js_toolchain=${PANE_JS_TOOLCHAIN_DIR:-$HOME/Library/Caches/pane/componentize-js}
if compgen -G "$js_toolchain/bin/*/toolchain.json" >/dev/null && command -v node >/dev/null; then
  develop_sample hello-ts "Hello TypeScript" dist/hello_ts.wasm src/index.ts 119 \
    'const GREETING: string = "%s from TypeScript";' 'const GREETING: string = 42;'
  develop_sample hello-js "Hello JavaScript" dist/hello_js.wasm src/index.js 128 \
    'const GREETING = "%s from JavaScript";' 'const GREETING = 42;'
else
  echo "skipped the JavaScript and TypeScript development smoke: no JS toolchain in $js_toolchain"
fi

# Disabling a required dependency: installed with the dependencies sample
# (whose install and data folder are this phase's own), the JavaScript
# operations sample is the first row of Manage extensions. Enter asks first,
# listing the Dependencies sample, which requires it, with Disable all and
# Cancel; Cancel changes nothing, Disable all disables both, and Enter again
# enables the JavaScript operations sample alone: the Dependencies sample
# stays disabled, on record too.
export PANE_DATA_DIR=$out/disable-dependents-data
rm -rf "$PANE_DATA_DIR"
start_pane --install target/guests/packages/sample-dependencies
key 36; sleep 3   # Install
for ((i = 0; i < 10; i++)); do key 125; done   # Manage extensions…
key 36; sleep 1
key 36; sleep 1   # disable JavaScript operations sample: asks first
capture 140-disable-dependents-asked.png
check 140-disable-dependents-asked.png aab4c0   # "Dependencies sample, which requires JavaScript operations sample · …"
key 125; key 36; sleep 1   # Cancel
capture 141-disable-dependents-cancelled.png   # both still enabled
key 36; sleep 1   # asks again
key 36; sleep 2   # Disable all 2
capture 142-disable-dependents-disabled.png
check 142-disable-dependents-disabled.png 9fd8a8   # "Disabled JavaScript operations sample and Dependencies sample, which requires it"
key 36; sleep 2   # enable JavaScript operations sample
capture 143-disable-dependents-enabled-alone.png
check 143-disable-dependents-enabled-alone.png 9fd8a8   # "Enabled JavaScript operations sample"; Dependencies sample stays disabled
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{140-disable-dependents-asked,141-disable-dependents-cancelled,142-disable-dependents-disabled,143-disable-dependents-enabled-alone}.png
stop_pane
[ "$(grep -c '"disabled": true' "$PANE_DATA_DIR/extensions/installed.json")" = 1 ] || { echo "not exactly the dependent left disabled"; exit 1; }

# Recovering from a crash of Pane's extension runtime (#17): the runtime is
# a thread of Pane, so the smoke has it panic on purpose through a fault
# file (PANE_TEST_RUNTIME_FAULTS; nothing else sets it). With the settings
# sample and the helper sample installed and the helper running, a crash
# ends the helper, keeps the saved note and restarts the runtime; Count (the
# settings sample's last item) then saves and loses its answer in a second
# crash, which stops the runtime: the count is not run again. Root search
# explains that nothing runs, Manage extensions shows why (its first rows),
# a disable still works, and Restart runs extensions again, Count only when
# asked. A data folder of its own keeps the rows in a known order.
export PANE_DATA_DIR=$out/runtime-crash-data
rm -rf "$PANE_DATA_DIR"
fault=$out/runtime-fault
rm -f "$fault" "$fault.tmp"
# Asks Pane to inject a fault; it takes the file within 100 ms.
inject() {
  printf %s "$1" >"$fault.tmp"
  mv "$fault.tmp" "$fault"
  for _ in $(seq 50); do [ -e "$fault" ] || break; sleep 0.1; done
  [ ! -e "$fault" ] || { echo "Pane did not take the fault"; exit 1; }
  sleep 2
}
# The count Count keeps in the settings sample's content.
count() {
  python3 - "$PANE_DATA_DIR/extensions/content.json" <<'PY'
import json, sys
packages = json.load(open(sys.argv[1], encoding="utf-8"))["packages"]
print(next((values["count"] for values in packages.values() if "count" in values), "none"))
PY
}
helpers_running() { pgrep -f "$PANE_DATA_DIR/extensions/packages/.*/pane-echo" >/dev/null; }
start_pane --install target/guests/packages/sample-helper
key 36; sleep 2   # Install
stop_pane
export PANE_TEST_RUNTIME_FAULTS=$fault
start_pane --install target/guests/packages/sample-settings
key 36; sleep 2   # Install; Greeting is selected
key 36; sleep 2   # open Greeting
for ((i = 0; i < 8; i++)); do key 125; done   # Count
key 36; sleep 2
capture 200-runtime-counted.png
check 200-runtime-counted.png 9fd8a8   # "Counted 1"
[ "$(count)" = 1 ] || { echo "Count did not count once"; exit 1; }
key 53; sleep 1
type_text helper; sleep 1
key 36; sleep 2   # open Helper sample
key 125; key 36; sleep 2   # Echo after waiting
helpers_running || { echo "the waiting helper is not running"; exit 1; }
capture 201-runtime-helper-waiting.png
check 201-runtime-helper-waiting.png d6c27a   # "Running…"
alive=$(find "$PANE_DATA_DIR/extensions/packages" -name pane-echo.alive | head -1)
[ -n "$alive" ] || { echo "the waiting helper does not beat"; exit 1; }
inject crash
capture 202-runtime-crashed.png
check 202-runtime-crashed.png f08c8c   # "Pane's extension runtime stopped unexpectedly and was started again; ..."
if helpers_running; then echo "the helper outlived the crashed runtime"; exit 1; fi
beats=$(stat -f %z "$alive"); sleep 0.5
[ "$(stat -f %z "$alive")" = "$beats" ] || { echo "the helper still beats after the crash"; exit 1; }
grep -q '"helper-wait": "started"' "$PANE_DATA_DIR/extensions/settings.json" || { echo "saved note lost"; exit 1; }
if grep -q '"helper-wait": "finished"' "$PANE_DATA_DIR/extensions/settings.json"; then echo "the stopped call finished"; exit 1; fi
key 53; sleep 1
type_text greet; sleep 1
key 36; sleep 2   # open Greeting in the restarted runtime
for ((i = 0; i < 8; i++)); do key 125; done   # Count
inject crash-before-answer:count
key 36; sleep 3   # counts, then the runtime crashes before answering
capture 203-runtime-stopped.png
check 203-runtime-stopped.png f08c8c   # the runtime stopped; its answer is lost
[ "$(count)" = 2 ] || { echo "Count did not run once before the crash"; exit 1; }
key 53; sleep 1
type_text greet; sleep 1
key 36; sleep 2   # Greeting: nothing runs
capture 204-runtime-refused.png
check 204-runtime-refused.png f08c8c   # "Extension runtime unavailable: it stopped after crashing ..."
key 53; sleep 1   # clears the query
for ((i = 0; i < 10; i++)); do key 125; done   # Manage extensions…
key 36; sleep 1
capture 205-runtime-manage.png   # Restart the extension runtime, Why the extension runtime stopped
key 125; key 36; sleep 1   # Why the extension runtime stopped
capture 206-runtime-details.png
check 206-runtime-details.png aab4c0   # the details
key 53; sleep 1   # back at its row
key 125; key 36; sleep 2   # disable Helper sample, the first package
capture 207-runtime-disabled.png
check 207-runtime-disabled.png 9fd8a8   # "Disabled Helper sample"
key 126; key 126; key 36; sleep 2   # Restart the extension runtime
capture 208-runtime-restarted.png
check 208-runtime-restarted.png 9fd8a8   # "Restarted the extension runtime"
[ "$(count)" = 2 ] || { echo "Count was run again without asking"; exit 1; }
key 53; sleep 1
type_text greet; sleep 1
key 36; sleep 2   # open Greeting
for ((i = 0; i < 8; i++)); do key 125; done   # Count
key 36; sleep 2
capture 209-runtime-counted-again.png
check 209-runtime-counted-again.png 9fd8a8   # "Counted 3"
[ "$(count)" = 3 ] || { echo "Count did not count once more"; exit 1; }
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{200-runtime-counted,202-runtime-crashed,203-runtime-stopped,204-runtime-refused,205-runtime-manage,206-runtime-details,207-runtime-disabled,208-runtime-restarted,209-runtime-counted-again}.png
stop_pane
unset PANE_TEST_RUNTIME_FAULTS
if helpers_running; then echo "a helper outlived Pane"; exit 1; fi
grep -q '"disabled": true' "$PANE_DATA_DIR/extensions/installed.json" || { echo "disable not recorded"; exit 1; }
if grep -q '"paused"' "$PANE_DATA_DIR/extensions/installed.json"; then echo "a package was paused for the runtime's crash"; exit 1; fi

# Uninstalling a required dependency: installed with the dependencies sample
# (whose install and data folder are this phase's own), the JavaScript
# operations sample's Uninstall row is the seventh of Manage extensions.
# Enter asks first, listing the Dependencies sample, which requires it, and
# each one's saved data, with Uninstall all keeping or deleting saved data
# and Cancel; Cancel changes nothing, Uninstall all 2 (keeping) uninstalls
# both, and installing the JavaScript operations sample again installs it
# alone: the Dependencies sample is not restored, on record too.
export PANE_DATA_DIR=$out/uninstall-dependents-data
rm -rf "$PANE_DATA_DIR"
start_pane --install target/guests/packages/sample-dependencies
key 36; sleep 3   # Install
for ((i = 0; i < 10; i++)); do key 125; done   # Manage extensions…
key 36; sleep 1
for ((i = 0; i < 6; i++)); do key 125; done   # Uninstall JavaScript operations sample
key 36; sleep 1   # asks first
capture 180-uninstall-dependents-asked.png
check 180-uninstall-dependents-asked.png aab4c0   # "Dependencies sample, which requires JavaScript operations sample · …"
key 125; key 125; key 36; sleep 1   # Cancel
capture 181-uninstall-dependents-cancelled.png   # both still installed
key 36; sleep 1   # asks again
key 36; sleep 3   # Uninstall all 2 and keep saved data
capture 182-uninstall-dependents-uninstalled.png
check 182-uninstall-dependents-uninstalled.png 9fd8a8   # "Uninstalled JavaScript operations sample and Dependencies sample, which requires it; …"
stop_pane
[ "$(grep -c '"dir"' "$PANE_DATA_DIR/extensions/installed.json")" = 0 ] || { echo "not both uninstalled"; exit 1; }
start_pane --install target/guests/packages/sample-operations-js
key 36; sleep 3   # Install the dependency alone
for ((i = 0; i < 10; i++)); do key 125; done   # Manage extensions…
key 36; sleep 1
capture 183-uninstall-dependents-reinstalled-alone.png   # only the JavaScript operations sample is listed
check 183-uninstall-dependents-reinstalled-alone.png aab4c0
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{180-uninstall-dependents-asked,181-uninstall-dependents-cancelled,182-uninstall-dependents-uninstalled,183-uninstall-dependents-reinstalled-alone}.png
stop_pane
[ "$(grep -c '"dir"' "$PANE_DATA_DIR/extensions/installed.json")" = 1 ] || { echo "not the dependency alone reinstalled"; exit 1; }

# npm packages (#45), from a local registry on 127.0.0.1 serving the npm
# sample `cargo xtask guests` packed (scripts/npm_registry.py; nothing reaches
# the network), with a data folder of its own. Installing the local
# Dependencies from npm sample shows the npm package it requires and
# installs both; its command calls the npm package's greet operation. Then
# "Install extension from npm…" (root's second-to-last row) asks for the
# npm package in a form; naming the installed one offers Update, and its
# command runs: "Hello from the npm package".
export PANE_DATA_DIR=$out/npm-data
rm -rf "$PANE_DATA_DIR"
rm -f "$out/npm-registry.port"
python3 "$(dirname "$0")/npm_registry.py" target/guests/npm "$out/npm-registry.port" 2>>"$out/npm-registry.log" &
npm_registry_pid=$!
for _ in $(seq 50); do [ -s "$out/npm-registry.port" ] && break; sleep 0.1; done
[ -s "$out/npm-registry.port" ] || { echo "the local npm registry did not start"; exit 1; }
export PANE_NPM_REGISTRY=http://127.0.0.1:$(cat "$out/npm-registry.port")/
start_pane --install target/guests/packages/sample-dependencies-npm
capture 260-npm-dependency-preview.png
check 260-npm-dependency-preview.png aab4c0   # "Requires: Greeter from npm, installed with it from npm:@pane-samples/greeter"
key 36; sleep 3   # Install; Greet through an npm dependency is selected
capture 261-npm-dependency-installed.png
check 261-npm-dependency-installed.png 9fd8a8   # "Installed Dependencies from npm sample with Greeter from npm, which it requires"
key 36; sleep 3   # open it
key 36; sleep 3   # "Greet through the required greeter"
capture 262-npm-dependency-called.png
check 262-npm-dependency-called.png 9fd8a8   # "Hello, Pane, from the npm package"
key 53; sleep 1
for ((i = 0; i < 10; i++)); do key 125; done   # Manage extensions…, the last row
key 126; key 36; sleep 1   # Install extension from npm…
capture 263-npm-form.png
check 263-npm-form.png 8a96a3   # the form's hint line
type_text @pane-samples/greeter
key 36; sleep 3
capture 264-npm-preview.png
check 264-npm-preview.png aab4c0   # "Source: npm package @pane-samples/greeter", "npm version: 0.1.0, the latest", …
key 36; sleep 3   # Update; Greeter from npm is selected
capture 265-npm-updated.png
check 265-npm-updated.png 9fd8a8   # "Updated Greeter from npm to 0.1.0"
key 36; sleep 3   # open Greeter from npm
key 36; sleep 2   # "Say hello"
capture 266-npm-command-ran.png
check 266-npm-command-ran.png 9fd8a8   # "Hello from the npm package"
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{260-npm-dependency-preview,261-npm-dependency-installed,262-npm-dependency-called,263-npm-form,264-npm-preview,265-npm-updated,266-npm-command-ran}.png
stop_pane
kill "$npm_registry_pid"; wait "$npm_registry_pid" 2>/dev/null || true; npm_registry_pid=
unset PANE_NPM_REGISTRY
grep -q '"npm": "@pane-samples/greeter"' "$PANE_DATA_DIR/extensions/installed.json" || { echo "npm package not recorded"; exit 1; }
grep -q '"npmVersion": "0.1.0"' "$PANE_DATA_DIR/extensions/installed.json" || { echo "npm version not recorded"; exit 1; }
[ "$(grep -c '"dir"' "$PANE_DATA_DIR/extensions/installed.json")" = 2 ] || { echo "not both installed"; exit 1; }

# File search (#29): Files, a default extension (its data folder is this
# phase's own; Files is selected once installed, and Pane's own "Choose
# folder…" row is the first of its command). Enter on it would show the
# system's folder picker; the smoke names the folder in
# PANE_TEST_CHOOSE_FOLDER instead (a debug build's hook). The fixture folder's
# path has spaces, and a file in it has non-ASCII letters too; typing "plan"
# lists that file, selected, and Enter hands it to Pane's handler for files,
# which PANE_TEST_OPEN_FILE_LOG (a debug build's hook) makes record the path
# instead of asking /usr/bin/open, since any application that opened it would
# be the user's own. An executable script in the folder is found but refused.
# The fixture is in the system's temporary folder, so no screenshot shows a
# home path.
export PANE_DATA_DIR=$out/files-data
rm -rf "$PANE_DATA_DIR"
files_fixture=$(mktemp -d "${TMPDIR:-/tmp}/pane-smoke-files.XXXXXX")
files_folder="$files_fixture/Pane smoke files"
mkdir -p "$files_folder/notes"
printf 'plan\n' >"$files_folder/Résumé plan ü.txt"
printf 'todo\n' >"$files_folder/notes/todo.txt"
printf '#!/bin/sh\ntouch "%s/runner-ran"\n' "$files_fixture" >"$files_folder/notes/runner.sh"
chmod +x "$files_folder/notes/runner.sh"
rm -f "$out/opened-file.txt"
export PANE_TEST_CHOOSE_FOLDER=$files_folder PANE_TEST_OPEN_FILE_LOG=$out/opened-file.txt
start_pane --install target/guests/packages/files
key 36; sleep 2   # Install; Files is selected
key 36; sleep 3   # open Files; "Choose folder…" is selected
key 36; sleep 2   # the folder PANE_TEST_CHOOSE_FOLDER names
capture 220-files-folder-granted.png
check 220-files-folder-granted.png 9fd8a8   # "Files may now list “Pane smoke files”"
key 53; sleep 1
type_text 'plan'; sleep 3
capture 221-files-found.png
check 221-files-found.png 364355 3000   # the selected file row, "Résumé plan ü.txt"
key 36; sleep 3
capture 222-files-opened.png
check 222-files-opened.png 9fd8a8   # "Opened Résumé plan ü.txt"
[ -f "$out/opened-file.txt" ] || { echo "the handler for files was not asked to open anything"; exit 1; }
[ "$(cd "$(dirname "$(head -1 "$out/opened-file.txt")")" && pwd -P)/$(basename "$(head -1 "$out/opened-file.txt")")" = "$(cd "$files_folder" && pwd -P)/Résumé plan ü.txt" ] || { echo "the handler for files was not asked to open the found file"; exit 1; }
rm -f "$out/opened-file.txt"
key 53; sleep 1
type_text 'runner'; sleep 3
key 36; sleep 2
capture 223-files-program-refused.png
check 223-files-program-refused.png f08c8c   # "Could not open runner.sh: it is a program or script, ..."
[ ! -e "$out/opened-file.txt" ] || { echo "the script was handed to the handler"; exit 1; }
[ ! -e "$files_fixture/runner-ran" ] || { echo "the script ran"; exit 1; }
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{220-files-folder-granted,221-files-found,222-files-opened,223-files-program-refused}.png
stop_pane
unset PANE_TEST_CHOOSE_FOLDER PANE_TEST_OPEN_FILE_LOG
rm -rf "$files_fixture"

# Searching an online service inside its command: Package search, the Rust
# search sample, queries the fixture service (a made-up package registry on
# a free port of 127.0.0.1, set as the sample's address through its form;
# nothing leaves this computer), whose log lists each request. Typed into
# root search, "aurora" finds nothing and sends the service nothing. Opened,
# the command's own search field sends it: its results are listed, Enter
# shows a package's details. A search the service holds ("slow...") is stopped when the text
# changes: the service sees its client hang up and the newer results show.
# The service's own error, then the service stopped (offline), are errors in
# place of results; once it is back, searching works again: the extension
# was not paused. A data folder of its own keeps the rows in a known order.
export PANE_DATA_DIR=$out/search-data
rm -rf "$PANE_DATA_DIR"
cargo build --locked --quiet -p pane-core --example fixture_service
service_log=$out/fixture-service.log
service_pid=
service_port=0
# Starts the fixture service, the `$1`th time: first on a free port, which
# it prints, then on that same port again.
start_service() {
  target/debug/examples/fixture_service --port "$service_port" >>"$service_log" 2>&1 &
  service_pid=$!
  for _ in $(seq 50); do
    if [ "$(grep -c 'listening on' "$service_log" 2>/dev/null)" -ge "$1" ]; then
      service_port=$(sed -n 's#.*listening on http://127\.0\.0\.1:\([0-9]*\).*#\1#p' "$service_log" | tail -n 1)
      return
    fi
    kill -0 "$service_pid" 2>/dev/null || break
    sleep 0.1
  done
  echo "the fixture service did not start (see $service_log)"; exit 1
}
stop_service() { kill "$service_pid"; wait "$service_pid" 2>/dev/null || true; service_pid=; }
trap '[ -z "$service_pid" ] || kill "$service_pid" 2>/dev/null || true; [ -n "$pid" ] && kill "$pid" 2>/dev/null || true' EXIT
rm -f "$service_log"
start_service 1
start_pane --install target/guests/packages/sample-search
key 36; sleep 2   # Install; Package search is selected
capture 160-search-installed.png
check 160-search-installed.png 9fd8a8   # "Installed Search sample"
type_text aurora; sleep 2
capture 161-root-typed.png   # root search: "No results for “aurora”"
if grep -q '^GET' "$service_log"; then echo "root search reached the service"; exit 1; fi
key 53; sleep 1   # clears the query
type_text 'package search'; sleep 1
key 36; sleep 3   # open Package search
capture 162-command-opened.png   # its own list, its search field empty
check 162-command-opened.png 364355 3000   # its first row, selected
key 125; key 36; sleep 2   # Service address: its form
type_text "http://127.0.0.1:$service_port"
key 36; sleep 2   # Save
capture 163-service-set.png   # "Searching http://127.0.0.1:<port> from now on"
check 163-service-set.png 9fd8a8
key 53; sleep 1   # back to the command, its search field empty
type_text aurora; sleep 3
capture 164-search-results.png   # aurora-charts, selected, and aurora-cli
check 164-search-results.png 364355 3000
grep -q '^GET /search?q=aurora$' "$service_log" || { echo "the command's search did not reach the service"; exit 1; }
key 125; key 36; sleep 3   # aurora-cli's details
capture 165-details.png
check 165-details.png 9fd8a8   # "aurora-cli 0.9.3 (Apache-2.0): Command-line parsing with subcommands"
command_key a; type_text slow; sleep 2   # held by the service
command_key a; type_text ember; sleep 3
capture 166-newer-search.png   # ember-tz, not what "slow" would list
check 166-newer-search.png 364355 3000
grep -q '^ABANDONED /search?q=slow$' "$service_log" || { echo "the replaced search was not stopped"; exit 1; }
command_key a; type_text down; sleep 3
capture 167-service-error.png
check 167-service-error.png f08c8c   # "... The service answered 503: the registry is down for maintenance"
stop_service
command_key a; type_text basalt; sleep 3
capture 168-offline.png
check 168-offline.png f08c8c   # "... Could not reach the service at http://127.0.0.1:<port>: connection refused"
start_service 2
command_key a; type_text cobalt; sleep 3
capture 169-back-online.png   # cobalt-http, selected: not paused
check 169-back-online.png 364355 3000
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{161-root-typed,162-command-opened,163-service-set,164-search-results,165-details,166-newer-search,167-service-error,168-offline,169-back-online}.png
stop_pane
stop_service

# Scheduled tasks (#47): the background sample's Ticks declares a schedule
# (every minute). Installing it schedules nothing; its row in Manage
# extensions turns the schedule on, which runs the task at once in the
# background: the row shows its answer, and the count it keeps is 1. A run
# that waits (the sample's "Wait 10 seconds in each run") is stopped by
# disabling the package where it waits: it never notes "finished", and,
# enabled again, the row says why the run stopped, without running it again.
# Turned off, the row is off again, and the record forgets it. A data folder
# of its own keeps the rows in a known order: the package's four rows, then
# Schedule: Ticks.
export PANE_DATA_DIR=$out/schedule-data
rm -rf "$PANE_DATA_DIR"
# The value the background sample keeps under $1 in its content.
kept() {
  python3 - "$PANE_DATA_DIR/extensions/content.json" "$1" <<'PY'
import json, sys
try:
    packages = json.load(open(sys.argv[1], encoding="utf-8"))["packages"]
except FileNotFoundError:
    packages = {}
print(next((values[sys.argv[2]] for values in packages.values() if sys.argv[2] in values), "none"))
PY
}
start_pane --install target/guests/packages/sample-background
key 36; sleep 2   # Install; Ticks is selected
capture 320-schedule-installed.png
check 320-schedule-installed.png 9fd8a8   # "Installed Background sample ..."
[ "$(kept ticks)" = none ] || { echo "installing ran the scheduled task"; exit 1; }
type_text manage; sleep 1
key 36; sleep 1   # Manage extensions
for ((i = 0; i < 4; i++)); do key 125; done   # Schedule: Ticks
sleep 1
capture 321-schedule-off.png   # "Off · Run it in the background every minute"
check 321-schedule-off.png 364355 3000
key 36; sleep 3   # on: it runs at once
capture 322-schedule-on.png   # "On · Every minute · Last run: Ticked 1 times"
check 322-schedule-on.png 9fd8a8   # "Ticks runs every minute in the background from now on"
[ "$(kept ticks)" = 1 ] || { echo "turning the schedule on did not run it once"; exit 1; }
grep -q '"outcome": "answered"' "$PANE_DATA_DIR/extensions/schedules.json" || { echo "the run's answer was not recorded"; exit 1; }
key 53; sleep 1   # root search
type_text ticks; sleep 1
key 36; sleep 2   # open Ticks: "Ticked 1 times" first
for ((i = 0; i < 4; i++)); do key 125; done   # Wait 10 seconds in each run
key 36; sleep 2
capture 323-schedule-wait-chosen.png
check 323-schedule-wait-chosen.png 9fd8a8   # "The next runs wait 10 seconds, then count"
key 53; sleep 1   # root search
type_text manage; sleep 1
key 36; sleep 1   # Manage extensions
for ((i = 0; i < 4; i++)); do key 125; done   # Schedule: Ticks
key 36; sleep 2   # off
key 36; sleep 2   # on again: a run starts at once, and waits
capture 324-schedule-running.png   # "On · Every minute · Running now"
check 324-schedule-running.png 9fd8a8   # "Ticks runs every minute in the background from now on"
[ "$(kept tick-wait)" = started ] || { echo "the waiting run did not start"; exit 1; }
for ((i = 0; i < 4; i++)); do key 126; done   # Background sample
key 36; sleep 2   # disable it while its run waits
capture 325-schedule-disabled.png
check 325-schedule-disabled.png 9fd8a8   # "Disabled Background sample"
sleep 10   # longer than the run would have waited
[ "$(kept tick-wait)" = started ] || { echo "the stopped run went on"; exit 1; }
[ "$(kept ticks)" = 1 ] || { echo "the stopped run counted"; exit 1; }
key 36; sleep 2   # enable it again
for ((i = 0; i < 4; i++)); do key 125; done   # Schedule: Ticks
sleep 1
capture 326-schedule-stopped.png   # "On · Every minute · Last run stopped: Background sample was disabled"
check 326-schedule-stopped.png 364355 3000
[ "$(kept tick-wait)" = started ] || { echo "enabling ran the stopped run again"; exit 1; }
key 36; sleep 2   # off
capture 327-schedule-turned-off.png
check 327-schedule-turned-off.png 9fd8a8   # "Ticks no longer runs on a schedule"
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{321-schedule-off,322-schedule-on,323-schedule-wait-chosen,324-schedule-running,325-schedule-disabled,326-schedule-stopped,327-schedule-turned-off}.png
stop_pane
grep -q '"tasks": {}' "$PANE_DATA_DIR/extensions/schedules.json" || { echo "the schedule turned off is still recorded"; exit 1; }

# Continuing services (#48): the service sample's Heartbeat sets
# "service": true. Installing it starts nothing; its row in Manage extensions
# starts it, and it runs in the background, beating every second: the row
# shows its status, and the count it keeps grows. Disabling the package stops
# it where it waits (the count stops), enabling it starts it again at once,
# and stopping it on its row stops it for good (the record forgets it). A
# data folder of its own keeps the rows in a known order: the package's four
# rows, then Service: Heartbeat.
export PANE_DATA_DIR=$out/service-data
rm -rf "$PANE_DATA_DIR"
services=$PANE_DATA_DIR/extensions/services.json
start_pane --install target/guests/packages/sample-service
key 36; sleep 2   # Install; Heartbeat is selected
capture 330-service-installed.png
check 330-service-installed.png 9fd8a8   # "Installed Service sample ..."
[ "$(kept beats)" = none ] || { echo "installing started the service"; exit 1; }
type_text manage; sleep 1
key 36; sleep 1   # Manage extensions
for ((i = 0; i < 4; i++)); do key 125; done   # Service: Heartbeat
sleep 1
capture 331-service-stopped.png   # "Stopped · Start it to run in the background while Pane runs"
check 331-service-stopped.png 364355 3000
key 36; sleep 3   # start it
capture 332-service-running.png   # "Running · Beat N"
check 332-service-running.png 9fd8a8   # "Heartbeat runs in the background from now on"
first=$(kept beats)
[ "$first" != none ] || { echo "starting the service did not run it"; exit 1; }
grep -q '#heartbeat"' "$services" || { echo "the started service was not recorded"; exit 1; }
sleep 3
[ "$(kept beats)" -gt "$first" ] || { echo "the service did not go on beating"; exit 1; }
for ((i = 0; i < 4; i++)); do key 126; done   # Service sample
key 36; sleep 2   # disable it while its service waits
capture 333-service-disabled.png
check 333-service-disabled.png 9fd8a8   # "Disabled Service sample"
stopped=$(kept beats)
sleep 3   # three beats' time
[ "$(kept beats)" = "$stopped" ] || { echo "the disabled package's service beat on"; exit 1; }
key 36; sleep 3   # enable it again: its service starts again at once
for ((i = 0; i < 4; i++)); do key 125; done   # Service: Heartbeat
sleep 1
capture 334-service-restarted.png   # "Running · Beat N", counting on
check 334-service-restarted.png 364355 3000
[ "$(kept beats)" -gt "$stopped" ] || { echo "enabling did not start the service again"; exit 1; }
key 36; sleep 2   # stop it
capture 335-service-turned-off.png
check 335-service-turned-off.png 9fd8a8   # "Heartbeat was stopped"
stopped=$(kept beats)
sleep 3
[ "$(kept beats)" = "$stopped" ] || { echo "the stopped service beat on"; exit 1; }
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{331-service-stopped,332-service-running,333-service-disabled,334-service-restarted,335-service-turned-off}.png
stop_pane
grep -q '"services": \[\]' "$services" || { echo "the stopped service is still recorded"; exit 1; }

echo "screenshots in $out"
