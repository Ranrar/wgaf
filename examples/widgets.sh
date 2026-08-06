#!/usr/bin/env bash
#
# Driving widgets by name instead of by coordinate.
#
# Opens a window full of controls — buttons, text boxes, a label, a long list —
# and operates them the way a person would describe them: "the button called
# Save", not "the pixel at 412, 260". Nothing here presses a key or moves the
# mouse; the desktop's accessibility interface reaches the widget directly.
#
# That is the difference worth seeing. A coordinate stops being right the
# moment a window moves, a theme changes, or a list scrolls. A name does not.
#
# Run it with:
#
#     ./examples/widgets.sh

source "$(dirname "${BASH_SOURCE[0]}")/_preconditions.sh"

require_wayland_session
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

start_app accessibility-test
wait_for_window_on_screen "accessibility-test"

APP="accessibility-test"

# Looks an element up by its name, and prints the reference wgaf uses to act
# on it. Everything below works this way: find the thing, then act on the
# thing.
#
# Searching by name rather than by name *and* kind on purpose: what a toolkit
# calls a given control is its own business — the same button is a "button" to
# one and a "push button" to another — while the name is what you actually
# know. Add `--role` when a name alone is ambiguous, and use `wgaf a11y tree`
# to see what your application calls things.
#
# Prints nothing at all when there is no match, rather than a reference made
# of empty pieces that would look valid and fail confusingly later.
find_element() {
    wgaf --json a11y find --app "$APP" --name "$1" |
        jq -r 'if length == 0 then empty
               else (.[0].element | "\(.bus_name)#\(.object_path)") end'
}

# The application exports its accessible tree a moment after its window
# appears, so this waits for it rather than assuming.
#
# Asked repeatedly for the same reason as the window wait above: wgaf has no
# command yet that means "wait until this application is there". Keeping this
# kind of timing in a script is a stopgap, not the shape it should end up in.
waited=0
until [ "$(wgaf --json a11y list-apps |
    jq -r --arg a "$APP" '[.[] | select(.name == $a)] | length')" -ge 1 ]; do
    sleep 0.2
    waited=$((waited + 1))
    [ "$waited" -gt 50 ] && missing "the $APP application never appeared to the accessibility service.
  Check that accessibility is enabled for your session."
done
pass "the application is visible to the accessibility service"

# --- what is in the window ------------------------------------------------------

heading "Everything wgaf can see in the window"

# A tree, cut short — the window has a deliberately deep and wide structure, so
# printing all of it would bury the interesting part.
wgaf a11y tree --app "$APP" --max-depth 3 || true
pause

buttons="$(wgaf --json a11y find --app "$APP" --role button | jq -r 'length')"
say "found $buttons buttons by asking for that kind of control alone"

# --- pressing a button ----------------------------------------------------------

heading "Pressing a button by its name"

activate="$(find_element "wgaf activate")"
if [ -z "$activate" ]; then
    fail "could not find the button called 'wgaf activate'"
    finish
fi
say "the button called 'wgaf activate' is $activate"

before="$(report '.activate_count')"
wgaf a11y click "$activate"
say "clicked it — without knowing where it is on screen"

if wait_for_report_value '.activate_count' "$((before + 1))"; then
    pass "the application counted the activation"
else
    fail "the application never registered the click"
fi

# --- typing into a box by name ---------------------------------------------------

heading "Putting text in a box by its name"

entry="$(find_element "wgaf editable entry")"
wgaf a11y set-text "$entry" "set by name, not by typing"
say "set the entry's text directly through the accessibility interface"

if wait_for_report_value '.entry_text' "set by name, not by typing"; then
    pass "the application reports the new text"
else
    fail "the text never arrived — got '$(report '.entry_text')'"
fi

# --- a control that refuses -------------------------------------------------------

heading "A control that will not do what it is asked"

# Not every widget supports every operation, and wgaf says so rather than
# pretending. This entry is read-only, and the accessibility interface refuses
# to change it — which is the correct answer, not a failure of wgaf.
readonly_entry="$(find_element "wgaf read-only entry")"
before_text="$(report '.readonly_text')"

if wgaf a11y set-text "$readonly_entry" "this should not stick" 2>/dev/null; then
    say "the command reported success"
else
    pass "wgaf reported that the control refused the change"
fi

sleep 0.5
check "the read-only text is unchanged" "$before_text" "$(report '.readonly_text')"

# --- a control that disappears -----------------------------------------------------

heading "What happens when a widget goes away"

# References are to live widgets. When one is destroyed, the reference to it
# stops working — and wgaf says the element is gone rather than failing in some
# way that looks like a bug in your script.
disposable="$(find_element "wgaf disposable")"
remove="$(find_element "wgaf remove")"

say "found the disposable label, and the button that destroys it"
wgaf a11y click "$remove"

if wait_for_report_value '.disposable_present' "false"; then
    pass "the application reports the label is gone"
else
    fail "the label was never removed"
fi

if wgaf a11y info "$disposable" >/dev/null 2>&1; then
    fail "wgaf still claims to see a widget that no longer exists"
else
    pass "using the old reference now reports the element as gone"
fi

finish
