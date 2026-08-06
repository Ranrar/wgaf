#!/usr/bin/env bash
#
# The emergency stop: your Escape stops wgaf, wgaf's own Escape does not.
#
# Opens a window, types into it so you can see automation running, and then
# shows the difference between the two:
#
#   1. wgaf sends Escape — nothing stops, and nothing closes either. While
#      wgaf is running the desktop takes that key for the emergency stop
#      before any application sees it, so a script cannot dismiss a dialog
#      with it.
#   2. The dialog is closed the way that does work: by pressing its own button
#      through the accessibility interface.
#   3. You press Escape on your keyboard — everything stops at once.
#   4. You release it, and typing works again.
#
# **This one uses the normal wgaf name rather than a private one**, unlike the
# other examples, because the desktop's emergency-key handling watches for that
# specific daemon. So stop a running wgaf first if you have one:
#
#     systemctl --user stop wgaf-daemon
#
# It types for real and needs you at the keyboard. Run it with:
#
#     ./examples/handbrake.sh

source "$(dirname "${BASH_SOURCE[0]}")/_preconditions.sh"

require_wayland_session
require_uinput
require_extension
require_jq
require_wgaf_built

# Unique to this example: it has to *be* the daemon the desktop is watching.
if busctl --user list --no-legend 2>/dev/null | grep -q "org.wgaf.Daemon"; then
    missing "another wgaf is already running under the name this example needs.
  Stop it first:

      systemctl --user stop wgaf-daemon"
fi

# The emergency key has to be plain Escape for this example to make sense. That
# is the shipped default, so anything else is a local change.
schemadir=""
for candidate in ~/.local/share/gnome-shell/extensions/wgaf@wgaf.dev/schemas \
                 /usr/share/gnome-shell/extensions/wgaf@wgaf.dev/schemas; do
    [ -d "$candidate" ] && schemadir="$candidate" && break
done
binding="$(gsettings ${schemadir:+--schemadir "$schemadir"} \
    get org.gnome.shell.extensions.wgaf kill-switch 2>/dev/null || echo unknown)"
if [ "$binding" != "['Escape']" ]; then
    missing "the emergency key is set to $binding rather than plain Escape, so
  this example cannot demonstrate it. Put it back with:

      gsettings ${schemadir:+--schemadir $schemadir} reset org.gnome.shell.extensions.wgaf kill-switch"
fi

trap cleanup EXIT

# The real name, not a private one — see the note at the top.
start_daemon "" "org.wgaf.Daemon"
start_app input-test --dialog
wait_for_window_on_screen "wgaf input-test"

target="$(wgaf --json window list |
    jq -r '.[] | select(.title == "wgaf input-test") | .id')"

# Put the dialog beside its parent, using wgaf itself.
#
# The application cannot do this: a Wayland application is never told where it
# is and cannot ask to be put somewhere, so the desktop decides — and on a
# multi-monitor setup it may decide on a screen you are not looking at. Moving
# it here is the only way to be sure you can see both windows at once.
dialog_id="$(wgaf --json window list |
    jq -r '.[] | select(.title | contains("dialog")) | .id')"
if [ -n "$dialog_id" ]; then
    main_x="$(wgaf --json window list |
        jq -r '.[] | select(.title == "wgaf input-test") | .x')"
    main_y="$(wgaf --json window list |
        jq -r '.[] | select(.title == "wgaf input-test") | .y')"
    wgaf window move "$dialog_id" "$((main_x + 660))" "$main_y" >/dev/null
    say "moved the dialog next to the window it belongs to"
fi

# True while wgaf is refusing to synthesize anything.
handbrake_on() {
    [ "$(wgaf --json status | jq -r '.input_stopped')" = "true" ]
}

# --- automation is running --------------------------------------------------------

heading "Automation is running"

# Typing also brings wgaf's virtual keyboard into existence, which is what arms
# the emergency key. Escape only belongs to wgaf while there is something to
# stop — the rest of the time it is your applications' key as normal.
wgaf type "automation is running " --window "$target"
say "typed into the window, and wgaf now holds a keyboard of its own"

handbrake_on && { fail "the handbrake is already on before we started"; finish; }
pass "the handbrake is off"

# --- wgaf's own Escape --------------------------------------------------------------

heading "wgaf sends Escape — and the dialog stays open"

# This is the part worth understanding. While wgaf is running, the desktop
# takes Escape for the emergency stop before any application sees it. So a
# script cannot dismiss a dialog by sending Escape: the key stops nothing and
# closes nothing. Watch the dialog not move.
if [ "$(report '.dialog_open')" != "true" ]; then
    fail "the dialog was not open, so there is nothing to demonstrate"
    finish
fi

wgaf key press escape --window "$dialog_id"
wgaf key release escape --window "$dialog_id"
say "wgaf pressed and released Escape at the dialog"

if [ "$(report '.dialog_open')" = "true" ]; then
    pass "the dialog is still open — Escape never reached it"
else
    fail "the dialog closed, which is not what the desktop should allow while
        the emergency stop is armed"
fi

if handbrake_on; then
    fail "wgaf stopped itself — a script sending Escape would trip the
        handbrake every time"
else
    pass "and nothing stopped either: wgaf ignores its own Escape"
fi

# --- the supported way ----------------------------------------------------------------

heading "Closing the dialog the way that does work"

# Press the dialog's own button instead, by name, through the accessibility
# interface. That reaches the widget directly rather than going through the
# keyboard, so the emergency stop is not in the way — and it keeps working if
# the dialog moves or is restyled, which a keystroke does not.
waited=0
until [ "$(wgaf --json a11y list-apps |
    jq -r '[.[] | select(.name == "input-test")] | length')" -ge 1 ]; do
    sleep 0.2
    waited=$((waited + 1))
    if [ "$waited" -gt 50 ]; then
        fail "the application never appeared to the accessibility service"
        finish
    fi
done

close_button="$(wgaf --json a11y find --app input-test --role button --name "Close dialog" |
    jq -r 'if length == 0 then empty
           else (.[0].element | "\(.bus_name)#\(.object_path)") end')"

if [ -z "$close_button" ]; then
    fail "could not find the dialog's Close dialog button"
    finish
fi
say "found the dialog's own button — pressing it"

wgaf a11y click "$close_button"

if wait_for_report_value '.dialog_open' "false"; then
    pass "the dialog closed — this is the way to dismiss one while wgaf runs"
else
    fail "the dialog is still open"
fi

wgaf type "and still running " --window "$target"
pass "still typing, as it should be"

# --- your Escape ----------------------------------------------------------------------

heading "Now you press Escape"

printf '\n      Press Escape on your keyboard — any window will do.\n'
printf '      Watch the counter in the window as you do: the desktop may take\n'
printf '      your Escape for the emergency stop before the application ever\n'
printf '      sees it, in which case the counter will not move at all.\n'
printf '      Waiting'

waited=0
until handbrake_on; do
    printf '.'
    sleep 1
    waited=$((waited + 1))
    if [ "$waited" -ge 30 ]; then
        printf '\n'
        fail "no Escape arrived in 30 seconds. If you did press it, the
        emergency key may not be reaching the desktop's handler."
        finish
    fi
done
printf '\n'
pass "the handbrake engaged after ${waited}s — your Escape stopped wgaf"

# --- what stopping means ----------------------------------------------------------------

heading "What that means while it is on"

if wgaf type "this must not appear" --window "$target" 2>/dev/null; then
    fail "wgaf typed while stopped"
else
    pass "wgaf refuses to type at all while the handbrake is on"
fi

# --- releasing it ------------------------------------------------------------------------

heading "Releasing the handbrake"

printf '\n      Press Enter to release it.\n      '
read -r _

wgaf release
say "ran: wgaf release"

if handbrake_on; then
    fail "the handbrake is still on"
else
    pass "released — wgaf may synthesize again"
fi

# Releasing does not resume what was interrupted; it only allows new commands.
# Whatever was running when you pressed Escape stays stopped.
# Looked up again rather than reused: the id from the start of the run is not
# guaranteed to still mean anything by now.
target="$(wgaf --json window list |
    jq -r '.[] | select(.title == "wgaf input-test") | .id')"
if [ -n "$target" ]; then
    wgaf type "released" --window "$target"
    pass "typing works again"
else
    fail "the window is gone, so there is nothing left to type into"
fi

finish
