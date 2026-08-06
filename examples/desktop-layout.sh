#!/usr/bin/env bash
#
# Screens and workspaces: what your desktop is shaped like, and moving around it.
#
# Lists your monitors and checks that against what GNOME itself reports, then
# opens a real window and works through a full round trip:
#
#     add a workspace  ->  go to it  ->  call the window over  ->
#     go back  ->  call the window back  ->  remove the workspace
#
# Every result is checked against what the application says it observed, never
# against wgaf's own account of itself.
#
# The order is chosen so you can see it happen. Sending a window away while
# watching the workspace it leaves shows a window vanishing, which looks the
# same as a window closing. Standing on the destination and calling it over
# shows the window appear, and an appearance is unmistakable.
#
# This example never types or clicks, so it does not need permission to use the
# input device. It does switch workspaces, which takes your screen away and
# brings it back — that is what you are watching for. It puts you back on the
# workspace you started on, and removes any workspace it added, however it ends.
#
# Run it with:
#
#     ./examples/desktop-layout.sh

source "$(dirname "${BASH_SOURCE[0]}")/_preconditions.sh"

require_wayland_session
require_extension
require_jq
require_wgaf_built

command -v gdbus >/dev/null ||
    missing "gdbus is not installed, and this example uses it to ask GNOME
  directly what your monitors are — so that wgaf's answer can be checked
  against something other than wgaf. It ships with glib2."

# --- putting the session back ---------------------------------------------------
#
# This example is the first one that rearranges the session itself rather than
# just the windows in it, so it has more to undo than the others. Recorded
# before anything changes and restored by the trap, so Ctrl-C in the middle
# leaves you where you started rather than on some workspace the example made.
STARTING_WORKSPACE=""
ADDED_WORKSPACE=""

restore_workspaces() {
    # Removal first: it shifts indices, and STARTING_WORKSPACE was read before
    # anything was added, so switching first and then removing could land you
    # somewhere else again.
    if [ -n "$ADDED_WORKSPACE" ]; then
        wgaf workspace remove "$ADDED_WORKSPACE" >/dev/null 2>&1 || true
    fi
    if [ -n "$STARTING_WORKSPACE" ]; then
        wgaf workspace switch "$STARTING_WORKSPACE" >/dev/null 2>&1 || true
    fi
}

example_cleanup() {
    restore_workspaces
    cleanup
}
trap example_cleanup EXIT

start_daemon

# The workspace half of this example needs an extension new enough to have the
# workspace methods. Checked here rather than discovered halfway through, so a
# stale extension is reported before anything opens on screen. The monitor half
# below would work without it, but a half-run example is a confusing thing to
# hand someone.
require_extension_ready

# --- your screens ---------------------------------------------------------------

heading "The monitors making up your desktop:"

wgaf monitor list || true

# Checked against GNOME's own display configuration rather than against wgaf a
# second time. wgaf reads the same service, so this does not prove the service
# is right — what it proves is that wgaf's *arithmetic* on the raw reply is:
# every logical monitor GNOME describes comes out as exactly one record, none
# invented and none dropped. That arithmetic is where the interesting mistakes
# live, since a monitor's logical size has to be recovered from its mode,
# divided by its scale and rotated.
gnome_connectors="$(
    gdbus call --session --dest org.gnome.Mutter.DisplayConfig \
        --object-path /org/gnome/Mutter/DisplayConfig \
        --method org.gnome.Mutter.DisplayConfig.GetCurrentState 2>/dev/null |
        grep -oE "\(\('[A-Za-z0-9-]+', '" | grep -oE "'[A-Za-z0-9-]+'" |
        tr -d "'" | sort -u | tr '\n' ' '
)"
wgaf_connectors="$(wgaf --json monitor list | jq -r '.[].connector' | sort -u | tr '\n' ' ')"

check "wgaf lists the same monitors GNOME does" "$gnome_connectors" "$wgaf_connectors"

# A rotated or scaled monitor is the case worth looking at, so say so when there
# is one — the numbers above are its *logical* size, which is what every wgaf
# coordinate is in, and not the resolution printed on the box.
if [ "$(wgaf --json monitor list | jq '[.[] | select(.transform != 0 or .scale != 1)] | length')" -gt 0 ]; then
    say "one of those is rotated or scaled — the size shown is the logical size,
      which is the one wgaf coordinates use"
fi

# The usable area is the part left after the top bar and any docks. It is the
# rectangle to size a window against, and it is reported as unknown rather than
# guessed at when the extension cannot supply it.
if [ "$(wgaf --json monitor list | jq '[.[] | select(.work_area != null)] | length')" -gt 0 ]; then
    pass "wgaf reports the usable area, so a window can be sized to fit under the top bar"
else
    say "no usable area reported — that needs the GNOME Shell extension, and the
      monitor list above came from GNOME directly"
fi

# --- what your workspaces look like ---------------------------------------------

heading "Your workspaces:"

wgaf workspace list || true
printf '\n'
wgaf workspace layout || true

STARTING_WORKSPACE="$(wgaf --json workspace layout | jq -r '.active')"
starting_count="$(wgaf --json workspace layout | jq -r '.n_workspaces')"
dynamic="$(wgaf --json workspace layout | jq -r '.dynamic')"

# Checked against the GNOME setting itself. Whether GNOME manages the workspace
# count is not something wgaf decides, so wgaf reporting it wrongly is exactly
# the kind of thing that would go unnoticed — and it changes what `add` means.
gnome_dynamic="$(gsettings get org.gnome.mutter dynamic-workspaces 2>/dev/null || echo unknown)"
check "wgaf agrees with GNOME about who manages the workspace count" \
    "$gnome_dynamic" "$dynamic"

# The grid has to be two numbers you can compute with. GNOME's own answer for
# the column count is -1, meaning "as many as needed" — this example printed
# `1 rows x -1 columns` the first time it was run, which is how that was found.
# A grid with a negative side cannot answer "which workspace is to the right",
# which is the only reason to report it.
rows="$(wgaf --json workspace layout | jq -r '.rows')"
columns="$(wgaf --json workspace layout | jq -r '.columns')"
if [ "$rows" -gt 0 ] && [ "$columns" -gt 0 ]; then
    pass "the workspace grid is ${rows}x${columns} — two numbers a script can use"
else
    fail "the workspace grid is ${rows}x${columns} — a side that is not positive is not a grid"
fi

# And it has to be big enough to hold what it describes, or a script walking it
# would never reach the last workspace.
if [ "$((rows * columns))" -ge "$starting_count" ]; then
    pass "the grid has room for all $starting_count workspace(s)"
else
    fail "the grid is ${rows}x${columns} but there are $starting_count workspaces"
fi

# --- a window to watch ----------------------------------------------------------

start_app window-test
wait_for_window_on_screen "wgaf window-test"

main_id="$(wgaf --json window list |
    jq -r '.[] | select(.title == "wgaf window-test") | .id')"

if [ -z "$main_id" ]; then
    fail "wgaf window list did not find the application's main window"
    finish
fi

heading "Giving the window focus, so losing it means something"

wgaf window focus "$main_id"
if wait_for_report_value '.windows[] | select(.role == "main") | .focused' "true"; then
    pass "the application reports its main window has focus"
else
    fail "the main window never took focus, so the rest of this example cannot mean anything"
    finish
fi

# --- somewhere to switch to -----------------------------------------------------

heading "Finding a second workspace to switch to"

if [ "$starting_count" -lt 2 ]; then
    # With dynamic workspaces GNOME keeps a spare at the end and there is
    # normally one already; with a fixed count there may genuinely be only one,
    # and then the example has to make its own.
    ADDED_WORKSPACE="$(wgaf --json workspace add | jq -r '.index')"
    if [ -z "$ADDED_WORKSPACE" ]; then
        fail "wgaf workspace add did not report the index of the workspace it added"
        finish
    fi
    say "added workspace $ADDED_WORKSPACE (this example removes it again at the end)"

    new_count="$(wgaf --json workspace layout | jq -r '.n_workspaces')"
    check "there is one more workspace than before" \
        "$((starting_count + 1))" "$new_count"
    target="$ADDED_WORKSPACE"
else
    say "there are already $starting_count, so nothing needs adding"
    # Any workspace that is not the one we are on.
    target="$(wgaf --json workspace list |
        jq -r --argjson active "$STARTING_WORKSPACE" \
            'first(.[] | select(.index != $active) | .index)')"
fi

# --- going there first, so the window can be watched arriving ---------------------
#
# The order matters for what you can actually see. Sending a window away while
# looking at the workspace it leaves shows you a window vanishing, which is the
# same picture as a window closing or crashing. Standing on the destination and
# calling it over shows you the window appear — and an appearance cannot be
# mistaken for anything else.

heading "Switching to workspace $target — an empty one, and the window is not here"

wgaf workspace switch "$target"

# The application speaking, not wgaf. A window on a workspace you are not
# looking at cannot hold keyboard focus, so it losing focus is independent
# evidence that the switch really happened — where asking wgaf which workspace
# is active would be asking the same code that just claimed to change it.
if wait_for_report_value '.windows[] | select(.role == "main") | .focused' "false"; then
    pass "the application reports its window lost focus, so the switch really happened"
else
    fail "the window still has focus, so the workspace never actually changed"
fi

# `wgaf workspace switch` does not return until the workspace is active, so this
# needs no waiting and no retry loop — which is the point of it working that way.
check "wgaf reports the new workspace as active as soon as the command returns" \
    "$target" "$(wgaf --json workspace layout | jq -r '.active')"

# --- calling the window over to where you are standing ----------------------------

heading "Bringing the window here — watch it appear on this workspace"

wgaf window move-to-workspace "$main_id" "$target"

# Asked for explicitly, because GNOME does not necessarily hand focus to an
# arriving window and this example must not depend on a courtesy.
wgaf window focus "$main_id"

if wait_for_report_value '.windows[] | select(.role == "main") | .focused' "true"; then
    pass "the application reports its window has focus — it is on this workspace now"
else
    fail "the window never arrived on workspace $target"
fi

# **This is the check that proves the window moved rather than the view.**
#
# Focusing a window that is on a *different* workspace makes GNOME take you to
# it — so if the move had silently done nothing, the focus above would have
# dragged the active workspace back to $STARTING_WORKSPACE. Still being here is
# only possible if the window is here too.
check "you are still on workspace $target — the window came to you, you did not go to it" \
    "$target" "$(wgaf --json workspace layout | jq -r '.active')"

# --- and back, in the same order ---------------------------------------------------

heading "Switching back to workspace $STARTING_WORKSPACE — the window stays behind"

wgaf workspace switch "$STARTING_WORKSPACE"

# It is on $target now, so leaving means it loses focus again — the mirror of
# the first switch, and evidence the window really did change workspace rather
# than following the view around.
if wait_for_report_value '.windows[] | select(.role == "main") | .focused' "false"; then
    pass "the application reports its window lost focus — it stayed on workspace $target"
else
    fail "the window followed the workspace switch, so it never really moved"
fi

heading "Calling the window back to workspace $STARTING_WORKSPACE"

wgaf window move-to-workspace "$main_id" "$STARTING_WORKSPACE"
wgaf window focus "$main_id"

if wait_for_report_value '.windows[] | select(.role == "main") | .focused' "true"; then
    pass "the application reports its window is back and has focus"
else
    fail "the window did not come back to workspace $STARTING_WORKSPACE"
fi

check "you are still on workspace $STARTING_WORKSPACE — it came back to you" \
    "$STARTING_WORKSPACE" "$(wgaf --json workspace layout | jq -r '.active')"

# --- putting it back ------------------------------------------------------------

heading "Tidying up"

# Done here as well as in the trap, so its result can be checked rather than
# only attempted. The trap stays for the paths that never reach this line.
if [ -n "$ADDED_WORKSPACE" ]; then
    wgaf workspace remove "$ADDED_WORKSPACE"
    removed="$ADDED_WORKSPACE"
    ADDED_WORKSPACE=""
    say "removed workspace $removed"

    check "the workspace count is back where it started" \
        "$starting_count" "$(wgaf --json workspace layout | jq -r '.n_workspaces')"
fi

check "you are back on the workspace you started on" \
    "$STARTING_WORKSPACE" "$(wgaf --json workspace layout | jq -r '.active')"

finish
