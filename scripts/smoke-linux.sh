#!/usr/bin/env bash
# Native GUI smoke on X11: starts a virtual X server (Xvfb), launches Pane,
# drives it with real key events and captures screenshots.
#
# Requires Xvfb, xdotool, Python 3 with Pillow (screenshot checks and, without
# ImageMagick's `import`, capture), plus a Vulkan driver (Mesa's lavapipe works without
# a GPU). Set PANE_XVFB / PANE_XDOTOOL to use binaries outside PATH. Pane keeps
# installed packages in <output-dir>/data, not the user's data folder.
# Usage: scripts/smoke-linux.sh <output-dir> [pane-binary]
set -euo pipefail
out=${1:-smoke}
pane=${2:-target/debug/pane}
xvfb=${PANE_XVFB:-Xvfb}
xdotool=${PANE_XDOTOOL:-xdotool}
mkdir -p "$out"
rm -rf "$out/data"
export PANE_DATA_DIR=$out/data
{ grep PRETTY_NAME /etc/os-release; uname -srm; } >"$out/system.txt"   # the tested OS and architecture

# Starts Xvfb on a display no other server uses, and uses it only once it
# is up: its socket appeared after it started and it is still running. A
# display another server has (the developer's own session, a parallel
# smoke) is never used: Xvfb exits there, and another number is tried.
xvfb_pid=
pane_pid=
npm_registry_pid=
cleanup() {
  [ -n "$pane_pid" ] && kill "$pane_pid" 2>/dev/null || true
  [ -n "$npm_registry_pid" ] && kill "$npm_registry_pid" 2>/dev/null || true
  [ -n "$xvfb_pid" ] && kill "$xvfb_pid" 2>/dev/null || true
}
trap cleanup EXIT
display=
for _ in $(seq 10); do
  number=$((90 + RANDOM % 100))
  socket=/tmp/.X11-unix/X$number
  [ -e "$socket" ] || [ -e "/tmp/.X$number-lock" ] && continue
  "$xvfb" ":$number" -screen 0 1280x800x24 -nolisten tcp 2>"$out/xvfb.log" &
  xvfb_pid=$!
  for _ in $(seq 50); do
    kill -0 "$xvfb_pid" 2>/dev/null || break
    [ -S "$socket" ] && break
    sleep 0.1
  done
  if kill -0 "$xvfb_pid" 2>/dev/null && [ -S "$socket" ]; then
    display=:$number
    break
  fi
  kill "$xvfb_pid" 2>/dev/null || true
  xvfb_pid=
done
[ -n "$display" ] || { echo "Xvfb did not start (see $out/xvfb.log)"; exit 1; }
export DISPLAY=$display
unset WAYLAND_DISPLAY
if command -v xdpyinfo >/dev/null; then
  xdpyinfo >/dev/null || { echo "Xvfb on $display does not answer"; exit 1; }
fi

capture() {
  if command -v import >/dev/null; then
    import -window root "$out/$1"
  else
    python3 -c 'import sys; from PIL import ImageGrab; ImageGrab.grab(xdisplay=sys.argv[1]).save(sys.argv[2])' \
      "$display" "$out/$1"
  fi
}
check() { python3 "$(dirname "$0")/check_screenshot.py" "$out/$1" "$2" ${3:+"$3"}; }
# Prints "x y": where the screenshot shows the given color.
locate() { python3 "$(dirname "$0")/check_screenshot.py" --locate "$out/$1" "$2"; }
# Clicks the primary button at screen position x y (screenshot pixels: the
# screenshot is of the whole X screen).
click_at() { "$xdotool" mousemove "$1" "$2" click 1; }

# Starts Pane with the given arguments and focuses its window.
start_pane() {
  "$pane" "$@" 2>>"$out/stderr.log" &
  pane_pid=$!
  window=
  for _ in $(seq 100); do
    window=$("$xdotool" search --onlyvisible --pid "$pane_pid" 2>/dev/null | head -1) && [ -n "$window" ] && break
    sleep 0.2
  done
  [ -n "$window" ] || { echo "Pane window did not appear"; exit 1; }
  sleep 2
}

stop_pane() {
  kill -0 "$pane_pid" || { echo "Pane exited during the smoke"; exit 1; }
  kill "$pane_pid"
  wait "$pane_pid" 2>/dev/null || true
  pane_pid=
}

start_pane
capture 1-root.png
check 1-root.png 8a96a3   # the hint line: text renders
"$xdotool" windowfocus --sync "$window"

# Open each sample command (Rust, JavaScript, TypeScript) and run an item.
for index in 0 1 2; do
  for ((i = 0; i < index; i++)); do "$xdotool" key Down; done
  "$xdotool" key Return; sleep 3
  capture "$((index + 2))-command-$index.png"
  "$xdotool" key Down key Return; sleep 2
  capture "$((index + 2))-result-$index.png"
  check "$((index + 2))-result-$index.png" 9fd8a8   # the guest's answer
  "$xdotool" key Escape; sleep 1
done
capture 5-back-to-root.png
# Each command must have answered from its own guest, not the same view twice.
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{2,3,4}-result-*.png

# The Rust command's form (its fifth item): submitting it empty is rejected
# and focus returns to the name, so typing there and choosing a greeting with
# Tab and Down makes the guest answer.
"$xdotool" key Return; sleep 3
for _ in 1 2 3 4; do "$xdotool" key Down; done
"$xdotool" key Return; sleep 1
capture 6-form.png
"$xdotool" key Return; sleep 2
capture 7-form-error.png
check 7-form-error.png f08c8c   # the rejected field's message
"$xdotool" type --delay 50 Ada
"$xdotool" key Tab key Down key Return; sleep 2
capture 8-form-result.png
check 8-form-result.png 9fd8a8   # the guest's answer
"$xdotool" key Escape key Escape; sleep 1
stop_pane

# Install the assembled Rust sample package (the folder the picker would
# return), then run its command. Root lists the three samples, the installed
# command, then the install and Manage extensions… rows.
start_pane --install target/guests/packages/sample-rust
"$xdotool" windowfocus --sync "$window"
capture 9-package.png
check 9-package.png aab4c0   # the package's identity and compatibility lines
"$xdotool" key Return; sleep 2
capture 10-installed.png
check 10-installed.png 9fd8a8   # "Installed Rust sample"
"$xdotool" key Return; sleep 3
"$xdotool" key Return; sleep 2
capture 11-installed-result.png
check 11-installed-result.png 9fd8a8   # the installed guest's answer
stop_pane

# The installed command is still listed after a restart.
start_pane
capture 12-restarted.png
check 12-restarted.png 8a96a3
[ -f "$out/data/extensions/installed.json" ] || { echo "no install record"; exit 1; }
"$xdotool" windowfocus --sync "$window"

# The Rust command's seventh item is declared for Windows only, its eighth
# for macOS and Linux only. Here the first is explained without running and
# the second runs.
"$xdotool" key Return; sleep 3
for _ in 1 2 3 4 5 6; do "$xdotool" key Down; done
"$xdotool" key Return; sleep 2
capture 13-windows-only.png
check 13-windows-only.png d6a36a   # the row's reason
check 13-windows-only.png f08c8c   # Linux: the reason as the error
"$xdotool" key Down key Return; sleep 2
capture 14-not-windows.png
check 14-not-windows.png 9fd8a8    # Linux: the guest's answer
"$xdotool" key Escape; sleep 1
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
  "platforms": ["windows", "macos"],
  "commands": [{ "id": "sample", "title": "Elsewhere sample", "component": "sample_rust.wasm" }]
}
JSON
start_pane --install "$out/elsewhere"
capture 15-no-compatible-package.png
check 15-no-compatible-package.png f08c8c   # "Not available on Linux: ..."
stop_pane

# Install the settings sample, save a choice with it, then disable it in
# Manage extensions. Root lists the three samples, Rust sample, Greeting, the
# install row, then Manage extensions… last; the extension list holds Rust
# sample, then Settings sample.
start_pane --install target/guests/packages/sample-settings
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install; Greeting is selected
"$xdotool" key Return; sleep 3   # open Greeting
"$xdotool" key Return; sleep 2   # "Use a formal greeting"
capture 16-setting-saved.png
check 16-setting-saved.png 9fd8a8   # "Saved the formal greeting"
"$xdotool" key Escape; sleep 1
for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # the last row
"$xdotool" key Return; sleep 1
"$xdotool" key Down key Return; sleep 2
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
"$xdotool" windowfocus --sync "$window"
capture 18-restarted-disabled.png
check 18-restarted-disabled.png 8a96a3
python3 "$(dirname "$0")/check_screenshot.py" --same "$out/12-restarted.png" "$out/18-restarted-disabled.png"
for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done
"$xdotool" key Return; sleep 1
"$xdotool" key Down key Return; sleep 2
capture 19-enabled.png
check 19-enabled.png 9fd8a8   # "Enabled Settings sample"
"$xdotool" key Escape; sleep 1
for ((i = 0; i < 4; i++)); do "$xdotool" key Down; done   # Greeting
"$xdotool" key Return; sleep 3
"$xdotool" key Down key Down key Return; sleep 2   # "Greet me"
capture 20-greeted.png
check 20-greeted.png 9fd8a8   # "Good day to you"
stop_pane

# Restarted, root lists Greeting again, after Rust sample.
start_pane
"$xdotool" windowfocus --sync "$window"

# The Rust command's color picker (its sixth item), which the guest draws:
# Right chooses purple, and a click on the dark green swatch chooses it. The
# chosen color fills its swatch and the preview, far more pixels than any
# other swatch covers.
"$xdotool" key Return; sleep 3
for _ in 1 2 3 4 5; do "$xdotool" key Down; done
"$xdotool" key Return; sleep 2
capture 21-color.png
check 21-color.png 1e88e5 3000   # blue, chosen when the view opens
"$xdotool" key Right; sleep 1
capture 22-color-key.png
check 22-color-key.png 8e24aa 3000   # purple
read -r x y < <(locate 22-color-key.png 1b5e20)
click_at "$x" "$y"; sleep 1
capture 23-color-click.png
check 23-color-click.png 1b5e20 3000   # dark green
"$xdotool" key Escape key Escape; sleep 1
stop_pane

# Root search: typing narrows root to the matching commands and Enter opens
# the best match. "typescr" matches only TypeScript sample, whose "Wait
# briefly" answers exactly as in step 4. A query that matches nothing shows
# no results, and Enter then opens nothing.
start_pane
"$xdotool" windowfocus --sync "$window"
"$xdotool" type --delay 50 typescr; sleep 1
capture 24-search.png
"$xdotool" key Return; sleep 3
"$xdotool" key Down key Return; sleep 2
capture 25-search-result.png
check 25-search-result.png 9fd8a8   # the TypeScript guest's answer
python3 "$(dirname "$0")/check_screenshot.py" --same "$out/4-result-2.png" "$out/25-search-result.png"
"$xdotool" key Escape; sleep 1
"$xdotool" type --delay 50 zzz; sleep 1
"$xdotool" key Return; sleep 1
capture 26-no-results.png
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{1-root,24-search,25-search-result,26-no-results}.png
stop_pane

# The calculator, a default extension: an expression typed into root search
# lists its answer first, selected, and Enter copies it. Pasting the copy
# over the query and typing on shows exactly the screen typing the whole
# expression shows, so the clipboard held the answer.
start_pane --install target/guests/packages/calculator
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install
"$xdotool" type --delay 50 '6*7'; sleep 2
capture 27-answer.png
check 27-answer.png 364355 3000   # the selected answer row
"$xdotool" key Return; sleep 1
capture 28-copied.png   # "Copied 42 to the clipboard"
"$xdotool" key ctrl+a; "$xdotool" type --delay 50 '42+1'; sleep 2
capture 29-typed.png
"$xdotool" key ctrl+a ctrl+v; "$xdotool" type --delay 50 '+1'; sleep 2
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
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install
capture 31-operations-target.png
check 31-operations-target.png 9fd8a8   # "Installed JavaScript operations sample"
stop_pane
start_pane --install target/guests/packages/sample-operations
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install; Call from Rust is selected
"$xdotool" key Return; sleep 3   # open Call from Rust
"$xdotool" key Return; sleep 2   # "Greet through another extension": its form
"$xdotool" type --delay 20 "local:$(realpath target/guests/packages/sample-operations-js)"
"$xdotool" key Tab; "$xdotool" type --delay 50 Rust
"$xdotool" key Return; sleep 5   # Greet
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
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install; Dev sample is selected
"$xdotool" key Return; sleep 3
"$xdotool" key Return; sleep 2   # "Say hello"
capture 33-dev-before.png
check 33-dev-before.png 9fd8a8   # "Hello from the Rust guest"
"$xdotool" key Escape; sleep 1
cp target/guests/sample_js.wasm "$out/dev/command.wasm"
for ((i = 0; i < 12; i++)); do "$xdotool" key Down; done   # the last row
"$xdotool" key Return; sleep 1
for ((i = 0; i < 11; i++)); do "$xdotool" key Down; done   # Reload Dev
"$xdotool" key Return; sleep 3
capture 34-reloaded.png
check 34-reloaded.png 9fd8a8   # "Reloaded Dev"
"$xdotool" key Escape; sleep 1
for ((i = 0; i < 8; i++)); do "$xdotool" key Down; done   # Dev sample
"$xdotool" key Return; sleep 3
"$xdotool" key Return; sleep 2   # "Say hello"
capture 35-dev-after.png
check 35-dev-after.png 9fd8a8   # "Hello from the JavaScript guest"
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out/33-dev-before.png" "$out/35-dev-after.png"
"$xdotool" key Escape; sleep 1

# A build that fails the install checks (here its component is missing) is
# not reloaded: the working code keeps running, exactly as before.
rm "$out/dev/command.wasm"
for ((i = 0; i < 12; i++)); do "$xdotool" key Down; done
"$xdotool" key Return; sleep 1
for ((i = 0; i < 11; i++)); do "$xdotool" key Down; done
"$xdotool" key Return; sleep 2
capture 36-not-reloaded.png
check 36-not-reloaded.png f08c8c   # "Dev was not reloaded: ..."
"$xdotool" key Escape; sleep 1
for ((i = 0; i < 8; i++)); do "$xdotool" key Down; done
"$xdotool" key Return; sleep 3
"$xdotool" key Return; sleep 2
capture 37-still-running.png
python3 "$(dirname "$0")/check_screenshot.py" --same "$out/35-dev-after.png" "$out/37-still-running.png"
"$xdotool" key Escape; sleep 1

# A build whose start fails is reported with Retry, after Reload Dev; this
# one saves a setting and fails its first start only, so Retry starts it.
cp target/guests/failing_start.wasm "$out/dev/command.wasm"
for ((i = 0; i < 12; i++)); do "$xdotool" key Down; done
"$xdotool" key Return; sleep 1
for ((i = 0; i < 11; i++)); do "$xdotool" key Down; done
"$xdotool" key Return; sleep 3
capture 38-start-failed.png
check 38-start-failed.png f08c8c   # "Reloaded Dev, but it failed to start; ..."
"$xdotool" key Down key Return; sleep 3   # Retry starting Dev
capture 39-retried.png
check 39-retried.png 9fd8a8   # "Started Dev"
stop_pane
grep -q '"start-attempted": "yes"' "$out/data/extensions/settings.json" || { echo "the failed start's setting was not kept"; exit 1; }

# The settings sample keeps one value of each kind of data: its formal style
# (settings) and "Good day to you" (cache) are saved above; its fourth and
# fifth items save a note (content) and sign in (a local credential), and its
# sixth shows all four.
start_pane
"$xdotool" windowfocus --sync "$window"
for ((i = 0; i < 4; i++)); do "$xdotool" key Down; done   # Greeting
"$xdotool" key Return; sleep 3
for ((i = 0; i < 3; i++)); do "$xdotool" key Down; done
"$xdotool" key Return; sleep 2   # "Save a note"
"$xdotool" key Down key Return; sleep 2   # "Sign in"
"$xdotool" key Down key Return; sleep 2   # "Show what Pane keeps"
capture 40-kept.png
check 40-kept.png 9fd8a8   # every value, the cached greeting included
"$xdotool" key Escape; sleep 1
stop_pane
grep -q '"note": "Water the plants"' "$out/data/extensions/content.json" || { echo "note not saved"; exit 1; }
grep -q '"token": "sample-token"' "$out/data/extensions/credentials.json" || { echo "credential not saved"; exit 1; }
grep -q '"last-greeting": "Good day to you"' "$out/data/extensions/cache.json" || { echo "greeting not cached"; exit 1; }

# Clear the settings sample's cache in Manage extensions: its row follows the
# six package rows, their six Reload rows and "Clear cache of Rust sample". Pane asks first, then deletes only the cached
# greeting, without running the extension.
start_pane
"$xdotool" windowfocus --sync "$window"
for ((i = 0; i < 12; i++)); do "$xdotool" key Down; done   # the last row
"$xdotool" key Return; sleep 1
for ((i = 0; i < 13; i++)); do "$xdotool" key Down; done
"$xdotool" key Return; sleep 1   # "Clear cache of Settings sample"
capture 41-confirm-clear-cache.png
check 41-confirm-clear-cache.png aab4c0   # what is deleted and what is kept
"$xdotool" key Return; sleep 2   # "Clear cache"
capture 42-cache-cleared.png
check 42-cache-cleared.png 9fd8a8   # "Cleared the cache of Settings sample"
"$xdotool" key Escape; sleep 1
for ((i = 0; i < 4; i++)); do "$xdotool" key Down; done   # Greeting
"$xdotool" key Return; sleep 3
for ((i = 0; i < 5; i++)); do "$xdotool" key Down; done
"$xdotool" key Return; sleep 2   # "Show what Pane keeps"
capture 43-kept-after-clear.png
check 43-kept-after-clear.png 9fd8a8   # "... Cached greeting: none"
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out/40-kept.png" "$out/43-kept-after-clear.png"
"$xdotool" key Escape; sleep 1
stop_pane
if grep -q 'Good day to you' "$out/data/extensions/cache.json"; then echo "cache not cleared"; exit 1; fi
grep -q '"greeting-style": "formal"' "$out/data/extensions/settings.json" || { echo "setting lost"; exit 1; }
grep -q '"note": "Water the plants"' "$out/data/extensions/content.json" || { echo "note lost"; exit 1; }
grep -q '"token": "sample-token"' "$out/data/extensions/credentials.json" || { echo "credential lost"; exit 1; }

# Applications, a default extension: an installed application is found by
# name in root search and Enter opens it. The application is a desktop entry
# the smoke adds in an XDG_DATA_HOME of its own (for Pane only), whose
# program writes a marker file, so nothing else is started; Pane still
# searches the system's applications too.
apps=$(cd "$out" && pwd)/apps
rm -rf "$apps"
mkdir -p "$apps/data/applications"
cat >"$apps/data/applications/pane-smoke-app.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Pane Smoke App
Exec=sh -c "echo launched > '$apps/launched'"
EOF
XDG_DATA_HOME=$apps/data start_pane --install target/guests/packages/applications
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install
"$xdotool" type --delay 50 'pane smoke'; sleep 3
capture 44-application.png
check 44-application.png 364355 3000   # the selected application row
"$xdotool" key Return; sleep 3
capture 45-opened.png
check 45-opened.png 9fd8a8   # "Opened Pane Smoke App"
for _ in $(seq 50); do [ -f "$apps/launched" ] && break; sleep 0.2; done
[ -f "$apps/launched" ] || { echo "the application did not run"; exit 1; }
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{44-application,45-opened}.png
stop_pane

# Quicklinks, a default extension: installed, its command's form saves a
# quicklink (Quicklinks is selected once installed, and "Create quicklink" is
# its first item). After a restart, typing part of its name lists it,
# selected, and Enter opens its address with the system's link handler:
# xdg-open, with no desktop session and a script that records the address,
# instead of starting a browser, as the only handler for web links.
start_pane --install target/guests/packages/quicklinks
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install
"$xdotool" key Return; sleep 3   # open Quicklinks
"$xdotool" key Return; sleep 1   # Create quicklink
"$xdotool" type --delay 50 'Pane issues'
"$xdotool" key Tab
"$xdotool" type --delay 50 'https://example.com/pane-issues'
"$xdotool" key Return; sleep 2
capture 46-quicklink-saved.png
check 46-quicklink-saved.png 9fd8a8   # "Saved quicklink “Pane issues”"
"$xdotool" key Escape key Escape; sleep 1
stop_pane
printf '#!/bin/sh\necho "$1" >"%s/opened-link.txt"\n' "$out" >"$out/browser.sh"
chmod +x "$out/browser.sh"
rm -f "$out/opened-link.txt"
# Only the recording script may open the link. No desktop session may choose
# a browser: xdg-open would ask it (gio, kde-open, ...) for the user's.
# XDG_CONFIG_HOME and XDG_DATA_HOME of the smoke's own, replacing the user's,
# make the script the default for web links, and every way xdg-open finds a
# default checks those before the system's; BROWSER, its last resort, is the
# script too. XDG_DATA_DIRS and XDG_CONFIG_DIRS keep the system's folders:
# Pane's Vulkan driver is found there (/usr/share/vulkan/icd.d), and without
# one Pane has no window on CI.
unset XDG_CURRENT_DESKTOP XDG_SESSION_DESKTOP DESKTOP_SESSION GDMSESSION DBUS_SESSION_BUS_ADDRESS \
  GNOME_DESKTOP_SESSION_ID KDE_FULL_SESSION KDE_SESSION_VERSION MATE_DESKTOP_SESSION_ID
xdg=$(cd "$out" && pwd)/xdg
rm -rf "$xdg"
mkdir -p "$xdg/applications"
cat >"$xdg/applications/pane-smoke-browser.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Pane Smoke Browser
Exec=$(cd "$out" && pwd)/browser.sh %u
MimeType=x-scheme-handler/http;x-scheme-handler/https;
NoDisplay=true
EOF
printf '[Default Applications]\nx-scheme-handler/http=pane-smoke-browser.desktop\nx-scheme-handler/https=pane-smoke-browser.desktop\n' \
  >"$xdg/mimeapps.list"
cp "$xdg/mimeapps.list" "$xdg/applications/mimeapps.list"
export BROWSER="$(cd "$out" && pwd)/browser.sh" XDG_CONFIG_HOME="$xdg" XDG_DATA_HOME="$xdg"
if command -v xdg-mime >/dev/null; then
  handler=$(xdg-mime query default x-scheme-handler/https)
  [ "$handler" = pane-smoke-browser.desktop ] || { echo "web links would open with $handler, not the smoke's script"; exit 1; }
fi
start_pane
"$xdotool" windowfocus --sync "$window"
"$xdotool" type --delay 50 'pane iss'; sleep 2
capture 47-quicklink-found.png
check 47-quicklink-found.png 364355 3000   # the selected quicklink row
"$xdotool" key Return; sleep 3
capture 48-quicklink-opened.png
check 48-quicklink-opened.png 9fd8a8   # "Opened https://example.com/pane-issues"
[ "$(cat "$out/opened-link.txt")" = https://example.com/pane-issues ] || { echo "the link handler was not asked to open the quicklink"; exit 1; }
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{46-quicklink-saved,47-quicklink-found,48-quicklink-opened}.png
stop_pane

# Uninstall the settings sample, keeping its saved data: its row follows the
# eight Clear cache rows. Pane asks first, showing its saved data, and the first
# choice keeps its settings and content while its copy and credential go.
# Installing the same folder again finds its formal style and note, signed out.
start_pane
"$xdotool" windowfocus --sync "$window"
for ((i = 0; i < 20; i++)); do "$xdotool" key Down; done   # the last row
"$xdotool" key Return; sleep 1
for ((i = 0; i < 25; i++)); do "$xdotool" key Down; done
"$xdotool" key Return; sleep 1   # "Uninstall Settings sample"
capture 49-confirm-uninstall.png
check 49-confirm-uninstall.png aab4c0   # what is removed and the saved data
"$xdotool" key Return; sleep 2   # "Uninstall and keep saved data"
capture 50-uninstalled.png
check 50-uninstalled.png 9fd8a8   # "Uninstalled Settings sample; its settings and content are kept"
stop_pane
grep -q '"retained"' "$out/data/extensions/installed.json" || { echo "kept data not recorded"; exit 1; }
if grep -q 'sample-token' "$out/data/extensions/credentials.json"; then echo "credential not removed"; exit 1; fi
grep -q '"greeting-style": "formal"' "$out/data/extensions/settings.json" || { echo "setting not kept"; exit 1; }
grep -q '"note": "Water the plants"' "$out/data/extensions/content.json" || { echo "note not kept"; exit 1; }
start_pane --install target/guests/packages/sample-settings
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install; Greeting is selected
"$xdotool" key Return; sleep 3   # open Greeting
for ((i = 0; i < 5; i++)); do "$xdotool" key Down; done
"$xdotool" key Return; sleep 2   # "Show what Pane keeps"
capture 51-reinstalled.png
check 51-reinstalled.png 9fd8a8   # "Style: formal · Note: Water the plants · Signed in: no ..."
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out/43-kept-after-clear.png" "$out/51-reinstalled.png"
"$xdotool" key Escape; sleep 1
stop_pane
if grep -q '"retained"' "$out/data/extensions/installed.json"; then echo "retained record not dropped"; exit 1; fi

# Global hotkeys: in Manage extensions, the settings sample's command,
# Greeting, is given Ctrl+Alt+G by pressing it on its hotkey screen (its row
# follows the package's state, Reload, Clear cache and Uninstall rows).
# With Pane no longer focused, pressing the hotkey opens Greeting in Pane's
# window, also after a restart; once the extension is disabled, pressing it does nothing.
# A data folder of its own keeps the rows in a known order. Only the Xvfb
# display is touched: Pane's key grab is on DISPLAY, and WAYLAND_DISPLAY is
# unset for the whole smoke.
unfocus_pane() {   # focus the root window: no Pane window has focus
  "$xdotool" windowfocus "$("$xdotool" search --maxdepth 0 '.*' 2>/dev/null | head -1)"; sleep 1
  [ "$("$xdotool" getwindowfocus 2>/dev/null)" != "$window" ] || { echo "Pane still has focus"; exit 1; }
}
press_hotkey() { "$xdotool" key ctrl+alt+g; sleep 3; }
export PANE_DATA_DIR=$out/hotkeys-data
rm -rf "$PANE_DATA_DIR"
start_pane --install target/guests/packages/sample-settings
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install; Greeting is selected
for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Manage extensions…
"$xdotool" key Return; sleep 1
"$xdotool" key Down Down Down Down Return; sleep 1   # "Hotkey for Greeting"
capture 52-hotkey-screen.png
check 52-hotkey-screen.png aab4c0   # "Press the keys that should open Greeting ..."
"$xdotool" key ctrl+alt+g; sleep 2
capture 53-hotkey-assigned.png
check 53-hotkey-assigned.png 9fd8a8   # "Ctrl+Alt+G now opens Greeting"
"$xdotool" key Escape; sleep 1   # root search
unfocus_pane
capture 54-unfocused.png
press_hotkey
capture 55-hotkey-opened.png
check 55-hotkey-opened.png 364355 3000   # Greeting's first item, selected
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{53-hotkey-assigned,55-hotkey-opened}.png
stop_pane
grep -q '"ctrl+alt+g"' "$PANE_DATA_DIR/extensions/hotkeys.json" || { echo "hotkey not recorded"; exit 1; }
start_pane
unfocus_pane
press_hotkey
capture 56-hotkey-after-restart.png
check 56-hotkey-after-restart.png 364355 3000   # Greeting's first item, selected
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{54-unfocused,56-hotkey-after-restart}.png
"$xdotool" windowfocus --sync "$window"   # no window manager: Pane is focused here
"$xdotool" key Escape; sleep 1
for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Manage extensions…
"$xdotool" key Return; sleep 1
"$xdotool" key Return; sleep 2   # disable Settings sample
"$xdotool" key Escape; sleep 1
capture 57-disabled.png   # root search
unfocus_pane
press_hotkey
"$xdotool" windowfocus --sync "$window"; sleep 1
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
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install; Greeting is selected
"$xdotool" key Return; sleep 2   # open Greeting
for ((i = 0; i < 7; i++)); do "$xdotool" key Down; done   # Crash
for ((i = 0; i < 3; i++)); do "$xdotool" key Return; sleep 2; done
"$xdotool" type --delay 50 greet; sleep 1   # Greeting and its reason at the top on any window height
capture 59-paused.png
check 59-paused.png f08c8c   # "Settings sample crashed 3 times within 5 minutes and is paused ..."
check 59-paused.png d6a36a   # Greeting: "Settings sample is paused after an error; ..."
stop_pane
grep -q '"paused"' "$PANE_DATA_DIR/extensions/installed.json" || { echo "pause not recorded"; exit 1; }
start_pane
"$xdotool" windowfocus --sync "$window"
"$xdotool" type --delay 50 greet; sleep 1
capture 60-paused-after-restart.png
check 60-paused-after-restart.png d6a36a   # Greeting is still paused
"$xdotool" key Escape; sleep 1   # clears the query
for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Manage extensions…
"$xdotool" key Return; sleep 1
"$xdotool" key Down Down Down Return; sleep 1   # "Why Settings sample is paused"
capture 61-pause-details.png
check 61-pause-details.png aab4c0   # the details
"$xdotool" key Return; sleep 2   # Retry Settings sample
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
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return   # Install; Greeting is selected
wait_for "$registry" sample-settings present; sleep 1
"$xdotool" key Return; sleep 3   # open Greeting
for ((i = 0; i < 3; i++)); do "$xdotool" key Down; done
"$xdotool" key Return   # "Save a note"
wait_for "$PANE_DATA_DIR/extensions/content.json" '"note": "Water the plants"' present
"$xdotool" key Escape; sleep 1   # root search
for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Manage extensions…
"$xdotool" key Return; sleep 1
"$xdotool" key Down Down Down Return; sleep 1   # "Uninstall Settings sample"
"$xdotool" key Return   # "Uninstall and keep saved data"
wait_for "$registry" '"retained"' present; sleep 1
for ((i = 0; i < 40; i++)); do "$xdotool" key Down; done
"$xdotool" key Return; sleep 1   # "Delete retained data of Settings sample"
capture 63-confirm-delete-retained.png
check 63-confirm-delete-retained.png aab4c0   # what is kept and what is not touched
"$xdotool" key Down Return   # "Delete retained data"
wait_for "$registry" '"retained"' absent; sleep 1
capture 64-retained-deleted.png
check 64-retained-deleted.png 9fd8a8   # "Deleted the retained data of Settings sample"
stop_pane
if grep -q 'Water the plants' "$PANE_DATA_DIR/extensions/content.json"; then echo "note not deleted"; exit 1; fi
start_pane --install target/guests/packages/sample-settings
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return   # Install; Greeting is selected
wait_for "$registry" sample-settings present; sleep 1
"$xdotool" key Return; sleep 3   # open Greeting
for ((i = 0; i < 5; i++)); do "$xdotool" key Down; done
"$xdotool" key Return; sleep 2   # "Show what Pane keeps"
capture 65-reinstalled-empty.png
check 65-reinstalled-empty.png 9fd8a8   # "Style: none · Note: none · Signed in: no ..."
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out/51-reinstalled.png" "$out/65-reinstalled-empty.png"
"$xdotool" key Escape; sleep 1
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
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install; Echo is selected
for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Manage extensions…
"$xdotool" key Return; sleep 1
for ((i = 0; i < 5; i++)); do "$xdotool" key Down; done   # "Alias for Echo"
"$xdotool" key Return; sleep 1
"$xdotool" type --delay 50 'ec'
"$xdotool" key Return; sleep 2
capture 66-alias-saved.png
check 66-alias-saved.png 9fd8a8   # "Typing “ec” now finds Echo"
"$xdotool" key Down Return; sleep 2   # "Fallback: Echo"
capture 67-fallback-on.png
check 67-fallback-on.png 9fd8a8   # "Echo is now offered for any text typed in root search"
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{66-alias-saved,67-fallback-on}.png
"$xdotool" key Escape; sleep 1   # root search
"$xdotool" type --delay 50 'ec hello'; sleep 1
capture 68-alias-row.png
check 68-alias-row.png 364355 3000   # Echo, sending “hello”, selected
"$xdotool" key Return; sleep 3
capture 69-alias-answer.png
check 69-alias-answer.png 9fd8a8   # "Echo heard “hello”"
"$xdotool" key Escape; sleep 1   # clears the query
"$xdotool" type --delay 50 'zqx'; sleep 1
capture 70-fallback-listed.png   # "No results for “zqx”", then Echo, not selected
"$xdotool" key Down; sleep 1
capture 71-fallback-chosen.png
check 71-fallback-chosen.png 364355 3000   # Echo, now selected
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{70-fallback-listed,71-fallback-chosen}.png
"$xdotool" key Return; sleep 3
capture 72-fallback-answer.png
check 72-fallback-answer.png 9fd8a8   # "Echo heard “zqx”"
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{69-alias-answer,72-fallback-answer}.png
stop_pane
grep -q '"ec"' "$PANE_DATA_DIR/extensions/aliases.json" || { echo "alias not recorded"; exit 1; }
grep -q '#echo"' "$PANE_DATA_DIR/extensions/aliases.json" || { echo "fallback not recorded"; exit 1; }
start_pane
"$xdotool" windowfocus --sync "$window"
for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Manage extensions…
"$xdotool" key Return; sleep 1
"$xdotool" key Return; sleep 2   # disable Query sample
"$xdotool" key Escape; sleep 1
"$xdotool" type --delay 50 'ec hello'; sleep 1
capture 73-alias-disabled.png   # "No results for “ec hello”"
stop_pane
grep -q '"disabled": true' "$PANE_DATA_DIR/extensions/installed.json" || { echo "not disabled"; exit 1; }
export PANE_DATA_DIR=$out/aliases-empty-data
rm -rf "$PANE_DATA_DIR"
start_pane
"$xdotool" windowfocus --sync "$window"
"$xdotool" type --delay 50 'ec hello'; sleep 1
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
"$xdotool" windowfocus --sync "$window"
capture 75-dependencies-preview.png
check 75-dependencies-preview.png aab4c0   # "Requires: JavaScript operations sample, installed with it ..."
"$xdotool" key Return; sleep 3   # Install; Greet through dependencies is selected
capture 76-dependencies-installed.png
check 76-dependencies-installed.png 9fd8a8   # "Installed Dependencies sample with JavaScript operations sample, which it requires"
"$xdotool" key Return; sleep 3   # open Greet through dependencies
"$xdotool" key Return; sleep 5   # Greet through the required greeter
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
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install; Helper sample is selected
"$xdotool" key Return; sleep 2   # open Helper sample
"$xdotool" key Return; sleep 2   # Echo through the helper
capture 90-helper-echoed.png
check 90-helper-echoed.png 9fd8a8   # 'Echoed "hello from Pane" on Linux x86-64'
"$xdotool" key Down Down Return; sleep 3   # Echo within a second
capture 91-helper-cancelled.png
check 91-helper-cancelled.png 9fd8a8   # "Stopped the helper after one second"
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{90-helper-echoed,91-helper-cancelled}.png
if helpers_running; then echo "a cancelled helper is still running"; exit 1; fi
"$xdotool" key Up Return; sleep 2   # Echo after waiting
helpers_running || { echo "the waiting helper is not running"; exit 1; }
capture 92-helper-waiting.png
"$xdotool" key Escape; sleep 1   # root search; the helper keeps running
for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Manage extensions…
"$xdotool" key Return; sleep 1
"$xdotool" key Return; sleep 2   # disable Helper sample
capture 93-helper-disabled.png
check 93-helper-disabled.png 9fd8a8   # "Disabled Helper sample"
if helpers_running; then echo "the helper outlived its disabled package"; exit 1; fi
grep -q '"helper-wait": "started"' "$PANE_DATA_DIR/extensions/settings.json" || { echo "saved note lost"; exit 1; }
if grep -q '"helper-wait": "finished"' "$PANE_DATA_DIR/extensions/settings.json"; then echo "the stopped call finished"; exit 1; fi
stop_pane
if helpers_running; then echo "a helper outlived Pane"; exit 1; fi

# Quitting Pane while a helper runs ends it: with "Echo after waiting"
# running (the helper beats in pane-echo.alive in its folder of the managed
# copy), closing the window the way a window manager asks quits Pane, which
# ends the helper first. A data folder of its own again.
export PANE_DATA_DIR=$out/helper-quit-data
rm -rf "$PANE_DATA_DIR"
start_pane --install target/guests/packages/sample-helper
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install; Helper sample is selected
"$xdotool" key Return; sleep 2   # open Helper sample
"$xdotool" key Down Return; sleep 2   # Echo after waiting
helpers_running || { echo "the waiting helper is not running"; exit 1; }
capture 94-helper-before-quit.png
check 94-helper-before-quit.png d6c27a   # "Running…"
alive=$(find "$PANE_DATA_DIR/extensions/packages" -name pane-echo.alive | head -1)
[ -n "$alive" ] || { echo "the waiting helper does not beat"; exit 1; }
python3 "$(dirname "$0")/close_window.py" "$window"
for _ in $(seq 50); do kill -0 "$pane_pid" 2>/dev/null || break; sleep 0.1; done
if kill -0 "$pane_pid" 2>/dev/null; then echo "Pane did not quit when its window closed"; exit 1; fi
wait "$pane_pid" 2>/dev/null || true
pane_pid=
if helpers_running; then echo "a helper outlived Pane quitting"; exit 1; fi
beats=$(stat -c %s "$alive"); sleep 0.5
[ "$(stat -c %s "$alive")" = "$beats" ] || { echo "the helper still beats after Pane quit"; exit 1; }

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
  "$xdotool" key Down Down Down Return; sleep 3
  "$xdotool" key Return; sleep 2
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
  "$xdotool" windowfocus --sync "$window"
  "$xdotool" key Return; sleep 2   # Install
  for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Manage extensions…
  "$xdotool" key Return; sleep 1
  for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Develop <title>
  "$xdotool" key Return; sleep 2
  capture "$n-$sample-develop-started.png"
  check "$n-$sample-develop-started.png" 9fd8a8   # "Developing <title>: each save in ..."
  "$xdotool" key Escape; sleep 1
  say_hello
  capture "$((n + 1))-$sample-greeting-before.png"
  check "$((n + 1))-$sample-greeting-before.png" 9fd8a8   # "Hello from ..."
  "$xdotool" key Escape; sleep 1

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
  "$xdotool" key Escape; sleep 1

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
  "$xdotool" key Escape; sleep 1

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
  "$xdotool" key Escape; sleep 1

  # Stopped: a save builds nothing.
  for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Manage extensions…
  "$xdotool" key Return; sleep 1
  for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Stop developing <title>
  "$xdotool" key Return; sleep 2
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
js_toolchain=${PANE_JS_TOOLCHAIN_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/pane/componentize-js}
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
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 3   # Install
for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Manage extensions…
"$xdotool" key Return; sleep 1
"$xdotool" key Return; sleep 1   # disable JavaScript operations sample: asks first
capture 140-disable-dependents-asked.png
check 140-disable-dependents-asked.png aab4c0   # "Dependencies sample, which requires JavaScript operations sample · …"
"$xdotool" key Down Return; sleep 1   # Cancel
capture 141-disable-dependents-cancelled.png   # both still enabled
"$xdotool" key Return; sleep 1   # asks again
"$xdotool" key Return; sleep 2   # Disable all 2
capture 142-disable-dependents-disabled.png
check 142-disable-dependents-disabled.png 9fd8a8   # "Disabled JavaScript operations sample and Dependencies sample, which requires it"
"$xdotool" key Return; sleep 2   # enable JavaScript operations sample
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
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install
stop_pane
export PANE_TEST_RUNTIME_FAULTS=$fault
start_pane --install target/guests/packages/sample-settings
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install; Greeting is selected
"$xdotool" key Return; sleep 2   # open Greeting
for ((i = 0; i < 8; i++)); do "$xdotool" key Down; done   # Count
"$xdotool" key Return; sleep 2
capture 200-runtime-counted.png
check 200-runtime-counted.png 9fd8a8   # "Counted 1"
[ "$(count)" = 1 ] || { echo "Count did not count once"; exit 1; }
"$xdotool" key Escape; sleep 1
"$xdotool" type --delay 50 helper; sleep 1
"$xdotool" key Return; sleep 2   # open Helper sample
"$xdotool" key Down Return; sleep 2   # Echo after waiting
helpers_running || { echo "the waiting helper is not running"; exit 1; }
capture 201-runtime-helper-waiting.png
check 201-runtime-helper-waiting.png d6c27a   # "Running…"
alive=$(find "$PANE_DATA_DIR/extensions/packages" -name pane-echo.alive | head -1)
[ -n "$alive" ] || { echo "the waiting helper does not beat"; exit 1; }
inject crash
capture 202-runtime-crashed.png
check 202-runtime-crashed.png f08c8c   # "Pane's extension runtime stopped unexpectedly and was started again; ..."
if helpers_running; then echo "the helper outlived the crashed runtime"; exit 1; fi
beats=$(stat -c %s "$alive"); sleep 0.5
[ "$(stat -c %s "$alive")" = "$beats" ] || { echo "the helper still beats after the crash"; exit 1; }
grep -q '"helper-wait": "started"' "$PANE_DATA_DIR/extensions/settings.json" || { echo "saved note lost"; exit 1; }
if grep -q '"helper-wait": "finished"' "$PANE_DATA_DIR/extensions/settings.json"; then echo "the stopped call finished"; exit 1; fi
"$xdotool" key Escape; sleep 1
"$xdotool" type --delay 50 greet; sleep 1
"$xdotool" key Return; sleep 2   # open Greeting in the restarted runtime
for ((i = 0; i < 8; i++)); do "$xdotool" key Down; done   # Count
inject crash-before-answer:count
"$xdotool" key Return; sleep 3   # counts, then the runtime crashes before answering
capture 203-runtime-stopped.png
check 203-runtime-stopped.png f08c8c   # the runtime stopped; its answer is lost
[ "$(count)" = 2 ] || { echo "Count did not run once before the crash"; exit 1; }
"$xdotool" key Escape; sleep 1
"$xdotool" type --delay 50 greet; sleep 1
"$xdotool" key Return; sleep 2   # Greeting: nothing runs
capture 204-runtime-refused.png
check 204-runtime-refused.png f08c8c   # "Extension runtime unavailable: it stopped after crashing ..."
"$xdotool" key Escape; sleep 1   # clears the query
for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Manage extensions…
"$xdotool" key Return; sleep 1
capture 205-runtime-manage.png   # Restart the extension runtime, Why the extension runtime stopped
"$xdotool" key Down Return; sleep 1   # Why the extension runtime stopped
capture 206-runtime-details.png
check 206-runtime-details.png aab4c0   # the details
"$xdotool" key Escape; sleep 1   # back at its row
"$xdotool" key Down Return; sleep 2   # disable Helper sample, the first package
capture 207-runtime-disabled.png
check 207-runtime-disabled.png 9fd8a8   # "Disabled Helper sample"
"$xdotool" key Up Up Return; sleep 2   # Restart the extension runtime
capture 208-runtime-restarted.png
check 208-runtime-restarted.png 9fd8a8   # "Restarted the extension runtime"
[ "$(count)" = 2 ] || { echo "Count was run again without asking"; exit 1; }
"$xdotool" key Escape; sleep 1
"$xdotool" type --delay 50 greet; sleep 1
"$xdotool" key Return; sleep 2   # open Greeting
for ((i = 0; i < 8; i++)); do "$xdotool" key Down; done   # Count
"$xdotool" key Return; sleep 2
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
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 3   # Install
for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Manage extensions…
"$xdotool" key Return; sleep 1
for ((i = 0; i < 6; i++)); do "$xdotool" key Down; done   # Uninstall JavaScript operations sample
"$xdotool" key Return; sleep 1   # asks first
capture 180-uninstall-dependents-asked.png
check 180-uninstall-dependents-asked.png aab4c0   # "Dependencies sample, which requires JavaScript operations sample · …"
"$xdotool" key Down Down Return; sleep 1   # Cancel
capture 181-uninstall-dependents-cancelled.png   # both still installed
"$xdotool" key Return; sleep 1   # asks again
"$xdotool" key Return; sleep 3   # Uninstall all 2 and keep saved data
capture 182-uninstall-dependents-uninstalled.png
check 182-uninstall-dependents-uninstalled.png 9fd8a8   # "Uninstalled JavaScript operations sample and Dependencies sample, which requires it; …"
stop_pane
[ "$(grep -c '"dir"' "$PANE_DATA_DIR/extensions/installed.json")" = 0 ] || { echo "not both uninstalled"; exit 1; }
start_pane --install target/guests/packages/sample-operations-js
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 3   # Install the dependency alone
for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Manage extensions…
"$xdotool" key Return; sleep 1
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
"$xdotool" windowfocus --sync "$window"
capture 260-npm-dependency-preview.png
check 260-npm-dependency-preview.png aab4c0   # "Requires: Greeter from npm, installed with it from npm:@pane-samples/greeter"
"$xdotool" key Return; sleep 3   # Install; Greet through an npm dependency is selected
capture 261-npm-dependency-installed.png
check 261-npm-dependency-installed.png 9fd8a8   # "Installed Dependencies from npm sample with Greeter from npm, which it requires"
"$xdotool" key Return; sleep 3   # open it
"$xdotool" key Return; sleep 3   # "Greet through the required greeter"
capture 262-npm-dependency-called.png
check 262-npm-dependency-called.png 9fd8a8   # "Hello, Pane, from the npm package"
"$xdotool" key Escape; sleep 1
for ((i = 0; i < 10; i++)); do "$xdotool" key Down; done   # Manage extensions…, the last row
"$xdotool" key Up Return; sleep 1   # Install extension from npm…
capture 263-npm-form.png
check 263-npm-form.png 8a96a3   # the form's hint line
"$xdotool" type --delay 50 @pane-samples/greeter
"$xdotool" key Return; sleep 3
capture 264-npm-preview.png
check 264-npm-preview.png aab4c0   # "Source: npm package @pane-samples/greeter", "npm version: 0.1.0, the latest", …
"$xdotool" key Return; sleep 3   # Update; Greeter from npm is selected
capture 265-npm-updated.png
check 265-npm-updated.png 9fd8a8   # "Updated Greeter from npm to 0.1.0"
"$xdotool" key Return; sleep 3   # open Greeter from npm
"$xdotool" key Return; sleep 2   # "Say hello"
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
# lists that file, selected, and Enter opens it with the system's handler for
# files: xdg-open, with no desktop session, whose only handler for plain text
# is a script that records the path, so no program of the user's opens it.
# An executable script in the folder is found but refused. The fixture is
# outside the home folder, so no screenshot shows a home path.
export PANE_DATA_DIR=$out/files-data
rm -rf "$PANE_DATA_DIR"
files_fixture=$(mktemp -d /tmp/pane-smoke-files.XXXXXX)
files_folder="$files_fixture/Pane smoke files"
mkdir -p "$files_folder/notes"
printf 'plan\n' >"$files_folder/Résumé plan ü.txt"
printf 'todo\n' >"$files_folder/notes/todo.txt"
printf '#!/bin/sh\ntouch "%s/runner-ran"\n' "$files_fixture" >"$files_folder/notes/runner.sh"
chmod +x "$files_folder/notes/runner.sh"
printf '#!/bin/sh\nprintf "%%s" "$1" >"%s/opened-file.txt"\n' "$(cd "$out" && pwd)" >"$out/file-opener.sh"
chmod +x "$out/file-opener.sh"
rm -f "$out/opened-file.txt"
unset XDG_CURRENT_DESKTOP XDG_SESSION_DESKTOP DESKTOP_SESSION GDMSESSION DBUS_SESSION_BUS_ADDRESS \
  GNOME_DESKTOP_SESSION_ID KDE_FULL_SESSION KDE_SESSION_VERSION MATE_DESKTOP_SESSION_ID
files_xdg=$(cd "$out" && pwd)/files-xdg
rm -rf "$files_xdg"
mkdir -p "$files_xdg/applications"
cat >"$files_xdg/applications/pane-smoke-file-opener.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Pane Smoke File Opener
Exec=$(cd "$out" && pwd)/file-opener.sh %f
MimeType=text/plain;
NoDisplay=true
EOF
printf '[Default Applications]\ntext/plain=pane-smoke-file-opener.desktop\n' >"$files_xdg/mimeapps.list"
cp "$files_xdg/mimeapps.list" "$files_xdg/applications/mimeapps.list"
export BROWSER="$(cd "$out" && pwd)/file-opener.sh" XDG_CONFIG_HOME="$files_xdg" XDG_DATA_HOME="$files_xdg"
if command -v xdg-mime >/dev/null; then
  handler=$(xdg-mime query default text/plain)
  [ "$handler" = pane-smoke-file-opener.desktop ] || { echo "text files would open with $handler, not the smoke's script"; exit 1; }
fi
export PANE_TEST_CHOOSE_FOLDER=$files_folder
start_pane --install target/guests/packages/files
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install; Files is selected
"$xdotool" key Return; sleep 3   # open Files; "Choose folder…" is selected
"$xdotool" key Return; sleep 2   # the folder PANE_TEST_CHOOSE_FOLDER names
capture 220-files-folder-granted.png
check 220-files-folder-granted.png 9fd8a8   # "Files may now list “Pane smoke files”"
"$xdotool" key Escape; sleep 1
"$xdotool" type --delay 50 'plan'; sleep 3
capture 221-files-found.png
check 221-files-found.png 364355 3000   # the selected file row, "Résumé plan ü.txt"
"$xdotool" key Return; sleep 4
capture 222-files-opened.png
check 222-files-opened.png 9fd8a8   # "Opened Résumé plan ü.txt"
[ -f "$out/opened-file.txt" ] || { echo "the handler for files was not asked to open anything"; exit 1; }
# Both sides resolved, as the same file.
[ "$(realpath "$(cat "$out/opened-file.txt")")" = "$(realpath "$files_folder/Résumé plan ü.txt")" ] || { echo "the handler for files was not asked to open the found file"; exit 1; }
rm -f "$out/opened-file.txt"
"$xdotool" key Escape; sleep 1
"$xdotool" type --delay 50 'runner'; sleep 3
"$xdotool" key Return; sleep 2
capture 223-files-program-refused.png
check 223-files-program-refused.png f08c8c   # "Could not open runner.sh: it is a program or script, ..."
[ ! -e "$out/opened-file.txt" ] || { echo "the script was handed to the handler"; exit 1; }
[ ! -e "$files_fixture/runner-ran" ] || { echo "the script ran"; exit 1; }
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{220-files-folder-granted,221-files-found,222-files-opened,223-files-program-refused}.png
stop_pane
unset PANE_TEST_CHOOSE_FOLDER
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
trap '[ -z "$service_pid" ] || kill "$service_pid" 2>/dev/null || true; cleanup' EXIT
rm -f "$service_log"
start_service 1
start_pane --install target/guests/packages/sample-search
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install; Package search is selected
capture 160-search-installed.png
check 160-search-installed.png 9fd8a8   # "Installed Search sample"
"$xdotool" type --delay 50 aurora; sleep 2
capture 161-root-typed.png   # root search: "No results for “aurora”"
if grep -q '^GET' "$service_log"; then echo "root search reached the service"; exit 1; fi
"$xdotool" key Escape; sleep 1   # clears the query
"$xdotool" type --delay 50 'package search'; sleep 1
"$xdotool" key Return; sleep 3   # open Package search
capture 162-command-opened.png   # its own list, its search field empty
check 162-command-opened.png 364355 3000   # its first row, selected
"$xdotool" key Down Return; sleep 2   # Service address: its form
"$xdotool" type --delay 20 "http://127.0.0.1:$service_port"
"$xdotool" key Return; sleep 2   # Save
capture 163-service-set.png   # "Searching http://127.0.0.1:<port> from now on"
check 163-service-set.png 9fd8a8
"$xdotool" key Escape; sleep 1   # back to the command, its search field empty
"$xdotool" type --delay 50 aurora; sleep 3
capture 164-search-results.png   # aurora-charts, selected, and aurora-cli
check 164-search-results.png 364355 3000
grep -q '^GET /search?q=aurora$' "$service_log" || { echo "the command's search did not reach the service"; exit 1; }
"$xdotool" key Down Return; sleep 3   # aurora-cli's details
capture 165-details.png
check 165-details.png 9fd8a8   # "aurora-cli 0.9.3 (Apache-2.0): Command-line parsing with subcommands"
"$xdotool" key ctrl+a; "$xdotool" type --delay 50 slow; sleep 2   # held by the service
"$xdotool" key ctrl+a; "$xdotool" type --delay 50 ember; sleep 3
capture 166-newer-search.png   # ember-tz, not what "slow" would list
check 166-newer-search.png 364355 3000
grep -q '^ABANDONED /search?q=slow$' "$service_log" || { echo "the replaced search was not stopped"; exit 1; }
"$xdotool" key ctrl+a; "$xdotool" type --delay 50 down; sleep 3
capture 167-service-error.png
check 167-service-error.png f08c8c   # "... The service answered 503: the registry is down for maintenance"
stop_service
"$xdotool" key ctrl+a; "$xdotool" type --delay 50 basalt; sleep 3
capture 168-offline.png
check 168-offline.png f08c8c   # "... Could not reach the service at http://127.0.0.1:<port>: connection refused"
start_service 2
"$xdotool" key ctrl+a; "$xdotool" type --delay 50 cobalt; sleep 3
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
"$xdotool" windowfocus --sync "$window"
"$xdotool" key Return; sleep 2   # Install; Ticks is selected
capture 320-schedule-installed.png
check 320-schedule-installed.png 9fd8a8   # "Installed Background sample ..."
[ "$(kept ticks)" = none ] || { echo "installing ran the scheduled task"; exit 1; }
"$xdotool" type --delay 50 manage; sleep 1
"$xdotool" key Return; sleep 1   # Manage extensions
for ((i = 0; i < 4; i++)); do "$xdotool" key Down; done   # Schedule: Ticks
sleep 1
capture 321-schedule-off.png   # "Off · Run it in the background every minute"
check 321-schedule-off.png 364355 3000
"$xdotool" key Return; sleep 3   # on: it runs at once
capture 322-schedule-on.png   # "On · Every minute · Last run: Ticked 1 times"
check 322-schedule-on.png 9fd8a8   # "Ticks runs every minute in the background from now on"
[ "$(kept ticks)" = 1 ] || { echo "turning the schedule on did not run it once"; exit 1; }
grep -q '"outcome": "answered"' "$PANE_DATA_DIR/extensions/schedules.json" || { echo "the run's answer was not recorded"; exit 1; }
"$xdotool" key Escape; sleep 1   # root search
"$xdotool" type --delay 50 ticks; sleep 1
"$xdotool" key Return; sleep 2   # open Ticks: "Ticked 1 times" first
for ((i = 0; i < 4; i++)); do "$xdotool" key Down; done   # Wait 10 seconds in each run
"$xdotool" key Return; sleep 2
capture 323-schedule-wait-chosen.png
check 323-schedule-wait-chosen.png 9fd8a8   # "The next runs wait 10 seconds, then count"
"$xdotool" key Escape; sleep 1   # root search
"$xdotool" type --delay 50 manage; sleep 1
"$xdotool" key Return; sleep 1   # Manage extensions
for ((i = 0; i < 4; i++)); do "$xdotool" key Down; done   # Schedule: Ticks
"$xdotool" key Return; sleep 2   # off
"$xdotool" key Return; sleep 2   # on again: a run starts at once, and waits
capture 324-schedule-running.png   # "On · Every minute · Running now"
check 324-schedule-running.png 9fd8a8   # "Ticks runs every minute in the background from now on"
[ "$(kept tick-wait)" = started ] || { echo "the waiting run did not start"; exit 1; }
for ((i = 0; i < 4; i++)); do "$xdotool" key Up; done   # Background sample
"$xdotool" key Return; sleep 2   # disable it while its run waits
capture 325-schedule-disabled.png
check 325-schedule-disabled.png 9fd8a8   # "Disabled Background sample"
sleep 10   # longer than the run would have waited
[ "$(kept tick-wait)" = started ] || { echo "the stopped run went on"; exit 1; }
[ "$(kept ticks)" = 1 ] || { echo "the stopped run counted"; exit 1; }
"$xdotool" key Return; sleep 2   # enable it again
for ((i = 0; i < 4; i++)); do "$xdotool" key Down; done   # Schedule: Ticks
sleep 1
capture 326-schedule-stopped.png   # "On · Every minute · Last run stopped: Background sample was disabled"
check 326-schedule-stopped.png 364355 3000
[ "$(kept tick-wait)" = started ] || { echo "enabling ran the stopped run again"; exit 1; }
"$xdotool" key Return; sleep 2   # off
capture 327-schedule-turned-off.png
check 327-schedule-turned-off.png 9fd8a8   # "Ticks no longer runs on a schedule"
python3 "$(dirname "$0")/check_screenshot.py" --distinct "$out"/{321-schedule-off,322-schedule-on,323-schedule-wait-chosen,324-schedule-running,325-schedule-disabled,326-schedule-stopped,327-schedule-turned-off}.png
stop_pane
grep -q '"tasks": {}' "$PANE_DATA_DIR/extensions/schedules.json" || { echo "the schedule turned off is still recorded"; exit 1; }

echo "screenshots in $out"
