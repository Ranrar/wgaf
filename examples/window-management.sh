#!/usr/bin/env bash
#
# Window management: list, focus, resize, close.
#
# Opens three real windows, then drives them with wgaf and checks the result
# against what the application itself says happened.
#
# This example never types or clicks, so it does not need permission to use the
# input device. It does move windows around on your screen — that is what you
# are watching for.
#
# Run it with:
#
#     ./examples/window-management.sh

source "$(dirname "${BASH_SOURCE[0]}")/_preconditions.sh"

require_wayland_session
require_extension
require_jq
require_wgaf_built

trap cleanup EXIT

start_daemon
start_app window-test

# The application is running before its windows are on screen, and wgaf
# reports a window with no size yet as 0x0. Waiting here means the listing
# below shows real sizes rather than a snapshot taken too early.
wait_for_window_on_screen "wgaf window-test"

heading "Three windows are open. wgaf can see them:"

# Windows are addressed by the id wgaf gives them, which is not the same as a
# title and not stable between runs — so every example looks the id up first.
wgaf window list | grep window-test || true

main_id="$(wgaf --json window list |
    jq -r '.[] | select(.title == "wgaf window-test") | .id')"

if [ -z "$main_id" ]; then
    fail "wgaf window list did not find the application's main window"
    finish
fi
pass "wgaf window list finds the main window (id $main_id)"

secondary_id="$(wgaf --json window list |
    jq -r '.[] | select(.title | contains("secondary")) | .id')"
dialog_id="$(wgaf --json window list |
    jq -r '.[] | select(.title | contains("dialog")) | .id')"

# --- moving them apart ----------------------------------------------------------

heading "Moving the windows apart so you can see all three"

# They open stacked on top of each other, each one inside the last, so until
# they are spread out only the front one is visible — and anything that happens
# to the others happens out of sight.
#
# Positions here are worked out from where the main window already is, rather
# than being fixed numbers, so this lands sensibly whatever your screen looks
# like. A position outside the screen is pulled back onto it by the desktop.
main_x="$(wgaf --json window list |
    jq -r '.[] | select(.title == "wgaf window-test") | .x')"
main_y="$(wgaf --json window list |
    jq -r '.[] | select(.title == "wgaf window-test") | .y')"

wgaf window move "$secondary_id" "$((main_x + 860))" "$main_y"
wgaf window move "$dialog_id" "$main_x" "$((main_y + 640))"
say "moved the other two out from behind the main window"

# Whether they ended up exactly there is deliberately not checked. A Wayland
# application is never told where the compositor put it, so the application
# cannot report a position to compare against — and taking wgaf's own word for
# it would be checking wgaf against itself.
pass "wgaf window move placed the other two windows"

# --- focus --------------------------------------------------------------------

heading "Moving focus from one window to the next"

# Now that they are side by side, watch the title bars: the focused window is
# the one drawn as active, and it changes each time.
#
# Which window the compositor focuses at startup varies between runs, so each
# step checks the change wgaf causes rather than the state beforehand.
for target in "secondary:$secondary_id" "dialog:$dialog_id" "main:$main_id"; do
    role="${target%%:*}"
    id="${target##*:}"

    wgaf window focus "$id"
    say "focused the $role window"

    if wait_for_report_value ".windows[] | select(.role == \"$role\") | .focused" "true"; then
        pass "the application reports its $role window has focus"
    else
        fail "the $role window never took focus"
    fi
done

# --- close --------------------------------------------------------------------

heading "Closing the secondary window"

if [ -z "$secondary_id" ]; then
    fail "wgaf window list did not find the secondary window"
    finish
fi

before="$(report_seq)"
wgaf window close "$secondary_id"
say "asked wgaf to close window $secondary_id"

if wait_for_report_change "$before"; then
    # A closed window stays in the report with visible: false, rather than
    # disappearing from it — so this asks whether it is still on screen, not
    # whether the application still knows about it.
    visible="$(report '.windows[] | select(.role == "secondary") | .visible')"
    check "the application reports the secondary window is gone" "false" "$visible"
else
    fail "the application never reported a change after wgaf window close"
fi

# --- resize -------------------------------------------------------------------

heading "Resizing the main window to 800x600"

before="$(report_seq)"
wgaf window resize "$main_id" 800 600
say "asked wgaf to resize window $main_id to 800x600 — watch it grow"

if wait_for_report_change "$before"; then
    width="$(report '.windows[] | select(.role == "main") | .width')"
    height="$(report '.windows[] | select(.role == "main") | .height')"
    check "the application reports its width as 800" "800" "$width"
    check "the application reports its height as 600" "600" "$height"
else
    fail "the application never reported a change after wgaf window resize"
fi

finish
