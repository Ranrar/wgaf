#!/usr/bin/env bash
#
# Window state: minimize, maximize, fullscreen, keep-above, show-on-all-
# workspaces, and raise/lower.
#
# Six pairs of commands that change what a window *is* rather than where it is.
# Each one is driven for real and then checked against what the application
# itself reports, wherever the application can see the difference.
#
# It does not type or click, so it needs no permission to use the input device.
# The one command here that would type is deliberately refused, and that
# refusal is the point of the step.
#
# It does move windows around, briefly cover your screen, and — for one step —
# switch workspace and switch back. That is what you are watching for.
#
# Run it with:
#
#     ./examples/window-state.sh

source "$(dirname "${BASH_SOURCE[0]}")/_preconditions.sh"

require_wayland_session
require_extension
require_jq
require_wgaf_built

trap cleanup EXIT

start_daemon
require_extension_ready
start_app window-test
wait_for_window_on_screen "wgaf window-test"

main_id="$(wgaf --json window list |
    jq -r '.[] | select(.title == "wgaf window-test") | .id')"

if [ -z "$main_id" ]; then
    fail "wgaf window list did not find the application's main window"
    finish
fi

# What wgaf currently reports about one of the main window's states. Used for
# the two states the application cannot see for itself — see the note above the
# always-on-top section.
window_state() {
    wgaf --json window list | jq -r --arg id "$main_id" \
        ".[] | select(.id == (\$id | tonumber)) | .$1"
}

# The main window's size according to the compositor, as WIDTHxHEIGHT.
window_size() {
    wgaf --json window list | jq -r --arg id "$main_id" \
        '.[] | select(.id == ($id | tonumber)) | "\(.width)x\(.height)"'
}

heading "Starting state"
wgaf window list | grep window-test || true
pass "the main window is id $main_id"

# --- minimize -----------------------------------------------------------------

heading "Minimizing the window, then bringing it back"

# The application cannot be asked whether it is minimized: a Wayland client is
# never told. What GTK does report is `suspended` — its own account of "not
# visible to the user" — and that is what changes here.
before="$(report_seq)"
wgaf window minimize "$main_id"
say "the main window should have vanished into the dock"

if wait_for_report_value '.windows[] | select(.role == "main") | .suspended' "true"; then
    pass "the application reports it is no longer visible to you"
else
    fail "the application never reported being hidden after wgaf window minimize"
fi

# --- typing at a minimized window is refused ----------------------------------

heading "Typing at it while it is minimized — which wgaf refuses"

# The reason this is worth a step of its own: a minimized window cannot hold the
# keyboard, so text aimed at one would land in whatever window does. wgaf checks
# first and stops, rather than typing into whatever you happen to be looking at.
#
# Nothing is typed here, and the window is left exactly as it was.
set +e
refusal="$(wgaf type --window "$main_id" "this must not be typed anywhere" 2>&1)"
refusal_status=$?
set -e

if [ "$refusal_status" -eq 4 ]; then
    pass "wgaf refused, and said which command fixes it"
    say "$refusal"
else
    fail "expected wgaf to refuse with exit code 4, got $refusal_status: $refusal"
fi

heading "Restoring it"

before="$(report_seq)"
wgaf window unminimize "$main_id"
say "the window should be back"

if wait_for_report_value '.windows[] | select(.role == "main") | .suspended' "false"; then
    pass "the application reports it is visible again"
else
    fail "the application never reported coming back after wgaf window unminimize"
fi

# --- maximize -----------------------------------------------------------------

heading "Maximizing the window, then restoring it"

# Both directions, always. GNOME can maximize a window sideways only from its
# own keyboard shortcuts, but it gives no way for another program to ask for
# that — so wgaf does not pretend to offer one.
original_size="$(window_size)"

wgaf window maximize "$main_id"
say "it should now fill the screen, stopping at the top bar"

if wait_for_report_value '.windows[] | select(.role == "main") | .maximized' "true"; then
    pass "the application reports it is maximized"
else
    fail "the application never reported being maximized"
fi

# The application only knows a yes/no — the Wayland protocol carries one
# "maximized" state and no size claim — so the compositor's own geometry is what
# says the window actually grew. A flag set without a resize behind it would
# pass the check above and fail this one.
maximized_size="$(window_size)"
if [ "$maximized_size" != "$original_size" ]; then
    pass "and it really did change size: $original_size -> $maximized_size"
else
    fail "the window reports maximized but is still $original_size"
fi

wgaf window unmaximize "$main_id"
say "and back to its own size"

if wait_for_report_value '.windows[] | select(.role == "main") | .maximized' "false"; then
    pass "the application reports it is no longer maximized"
else
    fail "the application never reported being unmaximized"
fi

check "it is back to the size it started at" "$original_size" "$(window_size)"

# --- fullscreen ---------------------------------------------------------------

heading "Fullscreen, which is not the same as maximized"

# A maximized window stops at the work area — the screen minus the top bar and
# any dock. A fullscreen one covers those too. If you are placing other windows
# around one, the difference is the height of the top bar.
wgaf window fullscreen "$main_id"
say "it should now cover everything, including the top bar"

if wait_for_report_value '.windows[] | select(.role == "main") | .fullscreen' "true"; then
    pass "the application reports it is fullscreen"
else
    fail "the application never reported going fullscreen"
fi

wgaf window unfullscreen "$main_id"

if wait_for_report_value '.windows[] | select(.role == "main") | .fullscreen' "false"; then
    pass "the application reports it left fullscreen"
else
    fail "the application never reported leaving fullscreen"
fi

# Leaving fullscreen must not have left it maximized instead. The two are
# different states and nothing here asked for the second one.
check "and it was not left maximized instead" "false" \
    "$(report '.windows[] | select(.role == "main") | .maximized')"

# --- show on all workspaces ---------------------------------------------------

heading "Showing the window on every workspace"

# This one has a real check available, and it is worth the trouble: stick the
# window, switch to another workspace, and the application should report itself
# still visible. A window that had not followed would be out of view there, and
# GTK would say so through the same `suspended` flag the minimize step used.
workspace_count="$(wgaf --json workspace list | jq -r 'length')"
original_workspace="$(wgaf --json workspace list | jq -r '.[] | select(.active) | .index')"
added_workspace=""

if [ "$workspace_count" -lt 2 ]; then
    wgaf workspace add >/dev/null
    added_workspace="yes"
    say "added a second workspace to demonstrate this with"
fi

other_workspace="$(wgaf --json workspace list |
    jq -r --argjson current "$original_workspace" \
        '[.[] | select(.index != $current) | .index] | first // empty')"

if [ -z "$other_workspace" ]; then
    say "only one workspace is available, so this step is being skipped"
else
    wgaf window stick "$main_id"
    say "the window is now on every workspace"

    wgaf workspace switch "$other_workspace"
    say "switched to workspace $other_workspace — the window should have come along"

    if wait_for_report_value '.windows[] | select(.role == "main") | .suspended' "false"; then
        pass "the application is still visible from another workspace"
    else
        fail "the window did not follow to workspace $other_workspace"
    fi

    wgaf window unstick "$main_id"
    say "unstuck it — and it stays here, on the workspace you are looking at"

    # **Unsticking leaves a window where you are, not where it came from.** A
    # window on every workspace is on this one too, so when it stops being on
    # all of them the one it keeps is the active one. That is the compositor's
    # behaviour, not a choice wgaf makes, and it is worth seeing: a script that
    # sticks a window, wanders off, and unsticks it has *moved* that window.
    check "wgaf reports it is no longer on all workspaces" "false" \
        "$(window_state on_all_workspaces)"
    check "and it is now on workspace $other_workspace, where you are" \
        "$other_workspace" "$(window_state workspace)"

    wgaf workspace switch "$original_workspace"
    say "switched back to workspace $original_workspace — the window stayed behind"

    # The proof that unsticking really detached it: it is no longer following.
    if wait_for_report_value '.windows[] | select(.role == "main") | .suspended' "true"; then
        pass "the application reports it is out of view, on the workspace it was left on"
    else
        fail "the window followed to workspace $original_workspace, so it is still stuck"
    fi

    # Put it back where the example found it, rather than leaving it on a
    # workspace the user never sent it to.
    wgaf window move-to-workspace "$main_id" "$original_workspace"
    say "moved it back to workspace $original_workspace"

    if wait_for_report_value '.windows[] | select(.role == "main") | .suspended' "false"; then
        pass "and it is where you left it"
    else
        fail "the window did not come back into view"
    fi
fi

if [ -n "$added_workspace" ]; then
    wgaf workspace remove "$other_workspace" >/dev/null 2>&1 || true
    say "removed the workspace this example added"
fi

# --- keep above, and stacking -------------------------------------------------

# The two states below are the ones the application genuinely cannot see. A
# Wayland client is told nothing about its stacking order, its layer, or which
# window is in front — there is no way to ask, in GTK or in the protocol. So
# unlike every check above, these read the state back from wgaf itself, which is
# a weaker thing to prove: it says the state was recorded, not that the desktop
# looks different. Watch the screen for the part the script cannot check.

heading "Keeping the window above the others"

wgaf window above "$main_id"
check "wgaf reports the window is kept above" "true" "$(window_state above)"
say "it should now stay in front even when you click another window"

wgaf window unabove "$main_id"
check "wgaf reports it is no longer kept above" "false" "$(window_state above)"

heading "Raising and lowering, with the windows overlapping"

# Moved on top of each other first, because stacking order is invisible when
# nothing overlaps — the demonstration only means anything if one window is
# actually hiding another.
main_x="$(wgaf --json window list |
    jq -r --arg id "$main_id" '.[] | select(.id == ($id | tonumber)) | .x')"
main_y="$(wgaf --json window list |
    jq -r --arg id "$main_id" '.[] | select(.id == ($id | tonumber)) | .y')"
secondary_id="$(wgaf --json window list |
    jq -r '.[] | select(.title | contains("secondary")) | .id')"

if [ -n "$secondary_id" ]; then
    wgaf window move "$secondary_id" "$((main_x + 40))" "$((main_y + 40))"
    say "the secondary window is now overlapping the main one"
fi

wgaf window raise "$main_id"
pass "raised the main window to the front"
say "the main window should be in front now"

wgaf window lower "$main_id"
pass "lowered it to the back"
say "and now behind — note that this did not move the keyboard, only the view"

# Nothing is checked for those two on purpose. There is no way to ask what the
# stacking order is: the application cannot see it, and wgaf does not report it
# either. Asserting on the reply to the command that caused it would be wgaf
# marking its own homework, so the example shows it and says so instead.

finish
