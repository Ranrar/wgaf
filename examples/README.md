# Examples

Runnable demonstrations of what wgaf does. Each one opens real windows on your
desktop, drives them with the `wgaf` command, and checks the result against
what the application itself reports.

```sh
./examples/window-management.sh
```

| Example | What it shows | Types or clicks? |
|---|---|---|
| `window-management.sh` | Listing, moving, focusing, resizing and closing real windows | no |
| `typing.sh` | Typing into a window you name, and holding a key across another | **yes** |
| `widgets.sh` | Operating buttons and text boxes by name rather than by coordinate | no |
| `mouse.sh` | Moving the pointer, clicking a button, and scrolling a list that really moves | **yes** |
| `handbrake.sh` | The emergency stop: your Escape stops wgaf, wgaf's own Escape does not | **yes** |

`handbrake.sh` is the one example that needs you at the keyboard rather than
away from it — it asks you to press Escape, and then to release it. It also
runs under the normal wgaf name instead of a private one, because the desktop's
emergency-key handling watches for that particular daemon, so stop a running
wgaf first (`systemctl --user stop wgaf-daemon`).

The ones marked **yes** press keys on your keyboard for real. Leave the
keyboard alone while they run — a few seconds — and note that they aim at a
window by name, so wgaf makes sure that window has focus before sending
anything and refuses rather than typing elsewhere if it cannot.

## What you need

A **GNOME desktop on Wayland**, with the wgaf GNOME Shell extension enabled,
and `jq` installed. Examples that type or click also need access to the input
device — the first-time setup in [`docs/installation.md`](../docs/installation.md)
covers it.

Each script checks all of this before anything opens on screen, and stops with
the command that fixes it if something is missing. Nothing gets built or
installed behind your back beyond compiling wgaf itself.

## Run them by hand, and watch

These are demonstrations, not a test suite. They take over your screen for a
few seconds — windows appear, move and close while they run — so run one when
you are not in the middle of something, and watch what happens rather than only
reading the output.

They deliberately pause between steps so there is time to see each one. If you
only want the results, turn that off:

```sh
WGAF_EXAMPLE_PACE=0 ./examples/typing.sh
```

or set it to any number of seconds to go slower still. It changes only how long
you wait: every check waits for the application to confirm what happened rather
than for a fixed time, so the pace never decides whether an example passes.

**A failure means wgaf is wrong, not that the example is flaky.** Every check
compares what wgaf was asked to do against what the application on the other
side actually observed. If a line says `FAIL`, something in wgaf did not do
what it claims.

## They leave your setup alone

Each example starts a wgaf daemon of its own, with its own settings, on its own
name. Your configuration is not read and not changed, a wgaf you already have
running is not disturbed, and everything the example started is shut down when
it ends — including if you press Ctrl-C.

## Why the application reports, rather than wgaf

Each example checks its work by reading a small file the application writes
describing what it saw: which of its windows have focus, what text arrived,
what was clicked.

It would be easier to ask wgaf to confirm its own work, and it would be worth
nothing. If wgaf's window handling had a bug, asking wgaf whether the window
moved could report success from the same broken code. Two independent accounts
have to agree instead — wgaf says what it did, and the application on the far
side says what it received.
