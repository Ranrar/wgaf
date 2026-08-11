#!/usr/bin/env bash
#
# Moving the pointer, clicking, and scrolling a list that really moves.
#
# Opens a window with a button and a long list, puts the pointer on them, and
# checks what the application received — including how far the list actually
# scrolled, rather than only that a scroll arrived.
#
# This example moves your mouse pointer and clicks for real, so it needs
# permission to use the input device. It will take the pointer away from
# whatever you are doing for a few seconds.
#
# Run it with:
#
#     ./examples/mouse.sh

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

# Where the window is, so the pointer can be aimed at things inside it. The
# application reports where its own widgets are, relative to its window, and
# wgaf reports where the window is on screen — together those give a real
# screen coordinate without either side guessing.
win_x="$(wgaf --json window list |
    jq -r '.[] | select(.title == "wgaf input-test") | .x')"
win_y="$(wgaf --json window list |
    jq -r '.[] | select(.title == "wgaf input-test") | .y')"
# The id as well, so the clicks and scrolls below can name the window they
# meant and be refused if the pointer is somewhere else by the time they run.
win_id="$(wgaf --json window list |
    jq -r '.[] | select(.title == "wgaf input-test") | .id')"
pass "the window is at ($win_x, $win_y), id $win_id"

# --- moving the pointer ----------------------------------------------------------

heading "Putting the pointer inside the window"

wgaf mouse move-to "$((win_x + 200))" "$((win_y + 120))"
say "moved the pointer to a point inside the window"

if wait_for_report_value '.pointer_in_window' "true"; then
    pass "the application can see the pointer over it"
else
    fail "the pointer never arrived over the window — did you move the mouse?
        wgaf sends pointer movement the same way your mouse does, so moving
        yours at the same time moves the same pointer somewhere else."
fi

# --- clicking a button by asking where it is ---------------------------------------

heading "Clicking the button, aimed by asking where it is"

# Hardcoding a coordinate would be asserting against this application's current
# layout rather than against wgaf, and would break silently the first time a
# margin changed — landing on empty space, reporting a click, and activating
# nothing. So the application is asked where its button is.
btn_x="$(report '.button_x')"
btn_y="$(report '.button_y')"
btn_w="$(report '.button_width')"
btn_h="$(report '.button_height')"

if [ "$btn_x" = "null" ]; then
    fail "the application has not laid out its button yet"
    finish
fi

# The centre, not the corner — a corner is one pixel from being outside.
target_x="$(jq -n --argjson w "$win_x" --argjson b "$btn_x" --argjson s "$btn_w" \
    '($w + $b + $s / 2) | round')"
target_y="$(jq -n --argjson w "$win_y" --argjson b "$btn_y" --argjson s "$btn_h" \
    '($w + $b + $s / 2) | round')"

before="$(report '.button_activations')"
wgaf mouse move-to "$target_x" "$target_y"

# `--window` is what makes the two commands above and below one safe step
# rather than two hopeful ones. Moving the pointer and clicking are separate
# commands, and it is your pointer in between — if you move the mouse now, the
# click would land on whatever you moved it to. Naming the window means wgaf
# checks what is under the pointer at the moment of the click and clicks
# nothing if it is not this window.
if wgaf mouse click left --window "$win_id"; then
    say "clicked at ($target_x, $target_y), the middle of the button"
else
    fail "the click was refused — the pointer was not over the window any more.
        That is the guard working: nothing was clicked. Leave the mouse alone
        while this runs."
    finish
fi

if wait_for_report_value '.button_activations' "$((before + 1))"; then
    pass "the application reports the button was activated"
else
    fail "the click did not activate the button"
fi

# --- scrolling something that moves ------------------------------------------------

heading "Scrolling the list"

# The list is longer than the space it is in, so there is somewhere for a
# scroll to go. Checking how far it moved rather than how many scroll events
# arrived: an event count goes up even when there is nothing to scroll, which
# proves the event was delivered and nothing more.
scroll_max="$(report '.scroll_maximum')"
say "the list can scroll $scroll_max pixels"

if [ "$(jq -n --argjson m "$scroll_max" '$m > 0')" != "true" ]; then
    fail "there is nothing to scroll — the list fits in the window"
    finish
fi

# Put the pointer over the list first: a scroll goes to whatever is under the
# pointer, not to whatever has keyboard focus.
wgaf mouse move-to "$((win_x + 200))" "$((win_y + 380))"
say "moved the pointer over the list"

# Confirm it is still there before scrolling, rather than assuming.
#
# Nothing keeps a pointer where wgaf put it: your own mouse moves the same
# pointer. The scrolls below name the window, so a lost pointer is refused
# rather than sent somewhere else — this check is here as well because it
# reports the problem in terms of the application, which is easier to act on
# than a refused command.
if ! wait_for_report_value '.pointer_in_window' "true"; then
    fail "the pointer is no longer over the window, so a scroll would go
        elsewhere — leave the mouse alone while this runs."
    finish
fi

start_pos="$(report '.scroll_position')"

# Several scrolls rather than one, so the list visibly travels instead of
# twitching. Each is a separate command, the same way you would send them.
scroll_repeatedly() {
    local direction="$1" times="$2" seq_before
    for _ in $(seq 1 "$times"); do
        seq_before="$(report_seq)"
        # Named, for the reason the click above is: a scroll that misses is
        # completely silent — no error, no visible sign, just a list that did
        # not move and an example blaming the wrong thing.
        wgaf mouse scroll 0 "$direction" --window "$win_id" || {
            fail "the scroll was refused — the pointer left the window."
            finish
        }
        # The application reports again once the view has actually moved, so
        # this waits for that rather than guessing how long scrolling takes.
        wait_for_report_change "$seq_before" || true
        pause
    done
}

scroll_repeatedly -3 6
say "scrolled down six times"

down_pos="$(report '.scroll_position')"

# Compared as "did it move down" rather than against an exact number: how far
# one notch of a wheel travels is the desktop's decision, not wgaf's.
if [ "$(jq -n --argjson e "$down_pos" --argjson s "$start_pos" '$e > $s')" = "true" ]; then
    pass "the list moved down, from $start_pos to $down_pos"
else
    fail "the list did not move — still at $down_pos"
fi

heading "Scrolling back up again"

# Positive is up here. Note the application sees the opposite sign: wgaf follows
# the kernel's convention, the toolkit follows its own, and wgaf does not quietly
# rewrite one into the other.
scroll_repeatedly 3 3
say "scrolled up three times"

up_pos="$(report '.scroll_position')"

if [ "$(jq -n --argjson e "$up_pos" --argjson s "$down_pos" '$e < $s')" = "true" ]; then
    pass "the list moved back up, from $down_pos to $up_pos"
else
    fail "the list did not move back up — still at $up_pos"
fi

finish
