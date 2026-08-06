#!/usr/bin/env bash
#
# Typing into a window you name, and checking it arrived.
#
# Opens a window with a text box, types into it, and reads back what the
# application actually received — not what wgaf believes it sent.
#
# This example types for real, on your keyboard, so it needs permission to use
# the input device. Every command here names the window it is aimed at, which
# is what keeps the text out of whatever else you have open: wgaf makes sure
# that window has focus before it sends anything, and refuses rather than
# typing somewhere else if it cannot.
#
# Run it with:
#
#     ./examples/typing.sh
#
# and leave the keyboard alone until it finishes.

source "$(dirname "${BASH_SOURCE[0]}")/_preconditions.sh"

require_wayland_session
require_uinput
require_extension
require_jq
require_wgaf_built

trap cleanup EXIT

start_daemon

# Checked here rather than before the daemon started, because the daemon is
# what knows which extension methods it needs. A session still running the
# extension it was logged in with fails this and says so, instead of dying
# partway through with a raw D-Bus error once windows are already on screen.
require_extension_ready

start_app input-test
wait_for_window_on_screen "wgaf input-test"

target="$(wgaf --json window list |
    jq -r '.[] | select(.title == "wgaf input-test") | .id')"

if [ -z "$target" ]; then
    fail "wgaf window list did not find the input-test window"
    finish
fi
pass "the text box is open in window $target"

# --- typing text ---------------------------------------------------------------

heading "Typing a line of text into window $target"

say "watch the text appear in the window"

# Sent as three commands rather than one so you can see it arrive a piece at a
# time. One command would be correct and instant — there is no way to make wgaf
# type slowly, and nothing here is trying to imitate a human at a keyboard.
before="$(report_seq)"
for part in "Hello " "from " "wgaf"; do
    wgaf type "$part" --window "$target"
    pause
done

if wait_for_report_change "$before"; then
    typed="$(report '.typed')"
    check "the application received exactly what was sent" "Hello from wgaf" "$typed"
else
    fail "the application never reported anything arriving"
fi

# Reading the text back is the check that matters, but the key events are what
# tell you *why* when it fails: no events at all means nothing arrived, while
# events with the wrong text means they arrived and the keyboard layout turned
# them into something else. Only the first of those is wgaf's fault.
keys="$(report '.key_event_count')"
if [ "$keys" -gt 0 ]; then
    pass "the application saw $keys key events"
else
    fail "the application saw no key events at all"
fi

# --- a capital letter, the long way ---------------------------------------------

heading "Holding shift to type a capital letter"

# `wgaf type` works out the keys for you. `wgaf key` is the level below it, for
# when you want to hold one key down across another — the four commands here
# are what typing a capital A actually involves.
#
# Every press needs its release. A key left held down stays held for the rest
# of your session, exactly as a physically stuck key would, so the releases run
# even if a press fails rather than the example stopping half way and leaving
# shift down on your keyboard.
before="$(report_seq)"
held=0
wgaf key press leftshift --window "$target" || held=1
wgaf key press a --window "$target" || held=1
wgaf key release a --window "$target" || held=1
wgaf key release leftshift --window "$target" || held=1
say "held shift, pressed A, released both"

[ "$held" -eq 0 ] || fail "one of the four key commands was refused"

if wait_for_report_change "$before"; then
    typed="$(report '.typed')"
    check "the capital letter arrived" "Hello from wgafA" "$typed"
else
    fail "the application never reported the capital letter"
fi

# --- naming a window that isn't there -------------------------------------------

heading "Aiming at a window that does not exist"

# The point of naming a window is that wgaf checks it before typing. Here that
# check fails, and the useful part is what does *not* happen: nothing is typed
# anywhere, rather than the text landing in whatever had focus instead.
before_text="$(report '.typed')"

if wgaf type "this must not be typed" --window 999999 2>/dev/null; then
    fail "wgaf accepted a window id that does not exist"
else
    pass "wgaf refused to type into a window that does not exist"
fi

# A brief pause rather than waiting for the application to report something:
# this step is checking that nothing happened, and there is no event to wait
# for when the expected outcome is silence.
sleep 0.5
after_text="$(report '.typed')"
check "nothing was typed anywhere" "$before_text" "$after_text"

finish
