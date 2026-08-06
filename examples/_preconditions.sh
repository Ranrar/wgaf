# Shared checks and setup for the wgaf examples.
#
# Sourced by every example script — not run on its own. It checks that this
# machine can actually run a demonstration before anything opens on screen, so
# a missing piece is reported as the thing to install rather than as a wgaf
# failure five steps later.
#
# It also starts a wgaf daemon of its own, on its own bus name, with its own
# settings. Your normal wgaf setup is left alone: nothing here reads or writes
# your real configuration, and the example's daemon disappears when the script
# ends.

set -euo pipefail

# Where the repository is, worked out from this file rather than from where you
# happened to run the script, so an example works from any directory.
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Everything the example creates lives here and is deleted on exit.
EXAMPLE_TMP="$(mktemp -d)"

# Filled in by start_daemon; used by the wgaf helper below.
EXAMPLE_BUS_NAME=""

# --- output ------------------------------------------------------------------

# Colour only when writing to a terminal, so piping to a file stays readable.
if [ -t 1 ]; then
    C_OK=$'\033[32m'; C_FAIL=$'\033[31m'; C_DIM=$'\033[2m'; C_OFF=$'\033[0m'
else
    C_OK=""; C_FAIL=""; C_DIM=""; C_OFF=""
fi

FAILURES=0

# How long to wait between steps so you can see what happened before the next
# one starts. These are demonstrations meant to be watched, and without this
# they are over before you have looked up.
#
# Set it to 0 for a quick run when you only want the results:
#
#     WGAF_EXAMPLE_PACE=0 ./examples/typing.sh
#
# This is presentation only. Nothing here depends on it — every example waits
# for the application to confirm what actually happened, never for a fixed
# amount of time, so setting this to 0 changes how the example looks and not
# whether it passes.
EXAMPLE_PACE="${WGAF_EXAMPLE_PACE:-1.5}"

pause() {
    [ "$EXAMPLE_PACE" = "0" ] || sleep "$EXAMPLE_PACE"
}

# A step that did what it was supposed to.
pass() {
    printf '%s  ok  %s%s\n' "$C_OK" "$C_OFF" "$1"
}

# A step that did not. The script keeps going so you see every result rather
# than only the first, and exits non-zero at the end.
fail() {
    printf '%sFAIL  %s%s\n' "$C_FAIL" "$C_OFF" "$1"
    FAILURES=$((FAILURES + 1))
}

# Narration between steps, so what you see on screen has a caption. Pauses
# afterwards so the thing just described is still on screen when you read it.
say() {
    printf '%s      %s%s\n' "$C_DIM" "$1" "$C_OFF"
    pause
}

# Announces the next thing the example is about to do, and gives you a moment
# to read it before it happens.
heading() {
    pause
    printf '\n%s\n' "$1"
    pause
}

# Compares what the application reported against what it should have been.
check() {
    local what="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        pass "$what"
    else
        fail "$what — expected '$expected', got '$actual'"
    fi
}

# The exit status for the whole example: zero only if every step passed.
#
# A failure here means wgaf did not do what it claims, not that a test is
# flaky. These drive real windows on your real desktop, so a result is a
# result.
finish() {
    printf '\n'
    if [ "$FAILURES" -eq 0 ]; then
        printf '%sEverything above worked.%s\n' "$C_OK" "$C_OFF"
        exit 0
    fi
    printf '%s%d step(s) failed.%s\n' "$C_FAIL" "$FAILURES" "$C_OFF"
    exit 1
}

# --- preconditions -----------------------------------------------------------

# Stops with one line naming what to install, rather than letting the example
# fail later in a way that looks like a wgaf bug.
missing() {
    printf '%sCannot run this example:%s %s\n' "$C_FAIL" "$C_OFF" "$1" >&2
    exit 1
}

require_wayland_session() {
    [ -n "${WAYLAND_DISPLAY:-}" ] ||
        missing "no Wayland session (WAYLAND_DISPLAY is unset). These examples
  drive real windows on a real GNOME Shell and cannot run without one."
}

require_uinput() {
    [ -w /dev/uinput ] ||
        missing "/dev/uinput is not writable, so wgaf cannot type or click.
  See the first-time setup in docs/installation.md: install the udev rule and
  join the 'input' group."
}

require_extension() {
    busctl --user status org.gnome.Shell.Extensions.Wgaf >/dev/null 2>&1 ||
        missing "the wgaf GNOME Shell extension is not running, so windows
  cannot be listed or focused. Enable it with:

      gnome-extensions enable wgaf@wgaf.dev

  and if it was only just installed, log out and back in — GNOME Shell cannot
  load an extension's code mid-session."
}

# Builds wgaf if needed and returns the paths. Building here rather than asking
# you to do it first means the example always runs what is in the tree now.
require_wgaf_built() {
    say "building wgaf..."
    cargo build --quiet --manifest-path "$REPO_ROOT/Cargo.toml" ||
        missing "wgaf failed to build. Fix that first — the error is above."
    WGAF="$REPO_ROOT/target/debug/wgaf"
    WGAF_DAEMON="$REPO_ROOT/target/debug/wgaf-daemon"
}

# The GTK applications the examples drive. Kept out of the main build because
# they are the only part of wgaf that needs GTK4 development packages.
require_test_app() {
    local name="$1"
    APP_BIN="$REPO_ROOT/tests/apps/target/debug/$name"
    if [ ! -x "$APP_BIN" ]; then
        say "building the $name application..."
        cargo build --quiet --manifest-path "$REPO_ROOT/tests/apps/Cargo.toml" ||
            missing "the '$name' application failed to build. It needs GTK4
  development packages — libgtk-4-dev on Debian/Ubuntu, gtk4-devel on Fedora."
    fi
    [ -x "$APP_BIN" ] ||
        missing "the '$name' application is missing from
  $APP_BIN even after building."
}

# --- the example's own daemon ------------------------------------------------

# Starts a wgaf daemon that belongs to this example alone.
#
# Its bus name includes this script's process id, so running an example never
# disturbs a wgaf you already have running, and two examples at once do not
# collide. Its configuration is written fresh here, so your own settings are
# neither read nor changed.
#
# Pass extra config.toml lines as the first argument to change a setting for
# one example.
start_daemon() {
    local extra_config="${1:-}"
    # A name of its own by default. One example needs the real one instead,
    # because the desktop's emergency-key handling watches for that specific
    # name — it passes it in here.
    EXAMPLE_BUS_NAME="${2:-org.wgaf.Example$$}"

    printf 'bus_name = "%s"\nlog_level = "warn"\ninput_device_name = "wgaf example %s"\n%s' \
        "$EXAMPLE_BUS_NAME" "$$" "$extra_config" > "$EXAMPLE_TMP/config.toml"
    printf '[capabilities]\n' > "$EXAMPLE_TMP/permissions.toml"
    chmod 600 "$EXAMPLE_TMP/config.toml" "$EXAMPLE_TMP/permissions.toml"

    "$WGAF_DAEMON" --config "$EXAMPLE_TMP/config.toml" \
        --permissions "$EXAMPLE_TMP/permissions.toml" \
        > "$EXAMPLE_TMP/daemon.log" 2>&1 &
    DAEMON_PID=$!

    # Wait for it to answer rather than sleeping a fixed amount: on a busy
    # machine a fixed wait is either too short to be reliable or longer than
    # anyone wants to sit through.
    local waited=0
    until "$WGAF" --bus-name "$EXAMPLE_BUS_NAME" ping >/dev/null 2>&1; do
        sleep 0.1
        waited=$((waited + 1))
        if [ "$waited" -gt 100 ]; then
            printf '%s\n' "$(cat "$EXAMPLE_TMP/daemon.log")" >&2
            missing "the example's wgaf daemon did not start. Its output is above."
        fi
    done
    say "wgaf daemon running on $EXAMPLE_BUS_NAME"
}

# Runs a wgaf command against this example's daemon.
#
# Every example calls wgaf through this, so what you see in the script is the
# command you would type yourself, minus the --bus-name that keeps the example
# separate from your own setup.
wgaf() {
    "$WGAF" --bus-name "$EXAMPLE_BUS_NAME" "$@"
}

# --- the application, and what it reports -------------------------------------

require_jq() {
    command -v jq >/dev/null ||
        missing "jq is not installed, and the examples use it to read what the
  application reported. Install it with your package manager, e.g.

      sudo apt install jq"
}

# Starts one of the GTK applications and waits until it has written its first
# report.
#
# The application writes a small JSON file describing what it observes — which
# windows it has, what was typed into it, what was clicked. That file is how
# these examples check results: wgaf does something, and the application says
# what it saw. Asking wgaf to confirm its own work would prove nothing.
start_app() {
    local name="$1"
    shift
    require_test_app "$name"
    REPORT="$EXAMPLE_TMP/$name.json"

    "$APP_BIN" --report "$REPORT" "$@" > "$EXAMPLE_TMP/$name.log" 2>&1 &
    APP_PID=$!

    local waited=0
    until [ -f "$REPORT" ]; do
        sleep 0.1
        waited=$((waited + 1))
        if [ "$waited" -gt 100 ]; then
            printf '%s\n' "$(cat "$EXAMPLE_TMP/$name.log")" >&2
            missing "the $name application did not start. Its output is above."
        fi
    done
    say "$name is running"
}

# Waits until the compositor has actually put a window on screen.
#
# Starting an application is not the same as having a window: the application
# is up, and reporting, a moment before anything has been drawn. Until then
# wgaf correctly reports the window as having no size yet, which would make an
# example look broken when it is only early. Takes the window's title, or any
# part of it.
#
# **This asks over and over because wgaf cannot yet be asked to wait.** There
# is no command that means "tell me when this window is ready", so the only
# thing available is to keep asking what it can see. Waiting belongs on wgaf's
# side of the line, not in a script — when it lives there, this goes away and
# every example gets shorter.
wait_for_window_on_screen() {
    local title="$1" waited=0
    until [ "$(wgaf --json window list |
        jq -r --arg t "$title" '[.[] | select(.title | contains($t)) | .width] | max // 0')" -gt 0 ]
    do
        sleep 0.1
        waited=$((waited + 1))
        if [ "$waited" -gt 100 ]; then
            missing "the '$title' window never appeared on screen."
        fi
    done
}

# Reads one value out of the application's report, using a jq expression.
report() {
    jq -r "$1" < "$REPORT"
}

# The report's change counter. Every time the application notices something, it
# rewrites the file with this number one higher.
report_seq() {
    report '.seq'
}

# Waits until the application has noticed something new since the given
# counter value.
#
# Needed because a wgaf command returns as soon as the compositor has accepted
# it, which is a moment before the application has drawn anything. Without this
# an example would read the report too early and report a failure that is only
# a race. Gives up after a few seconds rather than hanging.
wait_for_report_change() {
    local baseline="$1" waited=0
    while [ "$(report_seq)" -le "$baseline" ]; do
        sleep 0.1
        waited=$((waited + 1))
        [ "$waited" -gt 50 ] && return 1
    done
    return 0
}

# Waits until something the application reports has become what it should be.
#
# Better than waiting for "anything changed" when a single action produces more
# than one report: moving focus makes one window lose it before the next one
# gains it, so a check made after the first report would read the wrong answer
# and call it a failure. Gives up after a few seconds.
wait_for_report_value() {
    local filter="$1" expected="$2" waited=0
    until [ "$(report "$filter")" = "$expected" ]; do
        sleep 0.1
        waited=$((waited + 1))
        [ "$waited" -gt 50 ] && return 1
    done
    return 0
}

# --- cleanup -----------------------------------------------------------------

# Shuts down whatever the example started. Installed with 'trap' by each
# script, so it runs whether the example finished, failed, or you pressed
# Ctrl-C.
cleanup() {
    [ -n "${APP_PID:-}" ] && kill "$APP_PID" 2>/dev/null || true
    [ -n "${DAEMON_PID:-}" ] && kill "$DAEMON_PID" 2>/dev/null || true
    [ -n "${EXAMPLE_TMP:-}" ] && rm -rf "$EXAMPLE_TMP" || true
}
