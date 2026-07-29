# wgaf User Guide

Practical guide to using wgaf. For every command's exact flags, see the
[CLI reference](cli-reference.md). For installation, see the main
[README](../README.md).

## Before you start

Make sure `wgaf-daemon` is running (`wgaf-daemon &`, or installed as a
systemd service — see the README). Window commands additionally need the
GNOME Shell extension installed and enabled, also covered there.

To check all of that at once, run:

```sh
wgaf status
```

It reports whether the extension bridge, `/dev/uinput`, and the accessibility
bus are each usable, and prints what to do about any that aren't. It also
shows which capabilities your `permissions.toml` restricts, if any. Running it
changes nothing.

## Working with windows

See what's open, and note the id of the one you want:

```sh
wgaf window list
```

Focus, move, resize, or close it by id:

```sh
wgaf window focus 7
wgaf window move 7 100 100
wgaf window resize 7 900 700
wgaf window close 7
```

Window ids can change between sessions or after an app restarts — get a
fresh one from `window list` rather than reusing an old number.

## Typing and clicking

Keyboard/mouse input goes to whatever currently has focus on the whole
desktop, not a specific window you name — so focus the window first if you
want text to land in it:

```sh
wgaf window focus 7
wgaf type "hello world"
```

Press individual keys, e.g. to hold a modifier for a shortcut:

```sh
wgaf key press leftshift
wgaf key press a
wgaf key release a
wgaf key release leftshift
```

Move, click, or scroll the mouse:

```sh
wgaf mouse move 50 0
wgaf mouse click left
wgaf mouse scroll 0 -5
```

Two safety limits apply to everything in this section — one on how fast input
is produced, one on how much a single `wgaf type` may send. Both are generous
by default and you are unlikely to meet either by accident. See ["If automation
suddenly runs slowly"](#if-automation-suddenly-runs-slowly) below.

## Finding and clicking things by name

Instead of clicking at a screen position, look up the app and find the
element you want:

```sh
wgaf a11y list-apps
wgaf a11y find --app gtk4-demo --role "push button" --name Save
```

Take the element reference from the output (looks like
`:1.87#/org/a11y/atspi/accessible/1234`) and act on it:

```sh
wgaf a11y click :1.87#/org/a11y/atspi/accessible/1234
wgaf a11y set-text :1.87#/org/a11y/atspi/accessible/5678 "new text"
wgaf a11y focus :1.87#/org/a11y/atspi/accessible/1234
```

If you're not sure what to search for, browse the whole structure first:

```sh
wgaf a11y tree --app gtk4-demo
```

References only stay valid while that specific element is still on screen —
re-run `find`/`tree` for a fresh one if the UI has changed since you last
looked.

## Scripting with `--json`

Add `--json` to get machine-readable output instead of plain text — works on
either side of the subcommand:

```sh
wgaf --json window list
wgaf window list --json
```

## Configuration files

The daemon looks for two files automatically, in `$XDG_CONFIG_HOME/wgaf/`
(usually `~/.config/wgaf/`):

| File | What it does |
|---|---|
| `config.toml` | Daemon settings: bus name, log level, device name, input speed limit |
| `permissions.toml` | Per-capability policy: `Allow` / `Deny` / `Prompt` |

Keep both readable and writable by you alone (mode `600`). If either needs
attention, the daemon says so on startup with the command to fix it.

Within the policy file, permissions are an opt-in *restriction*: any
capability you don't mention is allowed, so the file is usually short.

Empty files select the defaults:

```sh
mkdir -p ~/.config/wgaf
: > ~/.config/wgaf/config.toml
printf '[capabilities]\n' > ~/.config/wgaf/permissions.toml
chmod 600 ~/.config/wgaf/config.toml ~/.config/wgaf/permissions.toml
```

`make install` places a template of each, if you don't already have one. They
list every setting and capability at its default value, so you can see the
whole picture and edit any line in place. Note that a written value is frozen
at whatever you wrote — comment a line out to go back to tracking the current
default.

Not sure which files are actually in use? Ask:

```sh
wgaf status
```

It names both paths, says whether each exists, and lists any capability that
isn't at its default — useful when a command was refused and you want to know
what did the refusing.

## If automation suddenly runs slowly

There is a speed limit on typing and clicking, set by
`input_max_events_per_second` in `config.toml` and counted in individual
keystrokes and clicks per second. The default of 3000 is far faster than any
real automation needs — a fast typist manages about 40.

It exists for one situation: a script with a loop bug that types or clicks
without stopping. Without a limit, the flood competes with your own keyboard
and mouse, and taking back control of the desktop becomes genuinely hard.

Going over the limit **slows commands down rather than failing them**, so a
long, legitimate automation still finishes — just paced. The daemon logs a
warning the first time this happens, so the reason is on the record.

If a script floods so badly that a command would have to wait more than half a
minute, that command fails instead, with a message saying the rate limit was
exceeded. At that point something is genuinely stuck in a loop, and waiting
would not help.

To raise the limit, or turn it off entirely:

```toml
# in ~/.config/wgaf/config.toml
input_max_events_per_second = 10000   # faster
input_max_events_per_second = 0       # no limit at all
```

Restart the daemon after changing it.

## Limiting how much one command can type

A separate setting caps a single `wgaf type` command:

```toml
input_max_type_text_chars = 4096   # the default
input_max_type_text_chars = 256    # a tighter guard
```

Anything longer is refused outright — nothing is typed at all, and the error
names the limit and the setting. Lower it if you would rather an oversized
paste fail than be typed into whichever window happens to be focused.

**`0` here means nothing may be typed, not "no limit"** — the opposite of the
speed setting above. That is deliberate, so that a mistaken `0` fails safe
instead of quietly removing the guard.

This is also the only limit on how much gets typed *in one go*. The speed
limit paces commands against each other; it does not slow down a single
command once it starts.

## What these limits are for

Both are safety nets, not security controls.

They protect you from your own scripts — a loop bug, a wrong variable, a paste
that turned out to be a hundred times longer than you thought. That is a real
and common way to lose control of a desktop, and these settings genuinely help.

## When a command gets denied

If a capability is set to `Deny` in `permissions.toml`, the command fails
immediately with "permission denied." If it's set to `Prompt`, a desktop
notification appears asking Allow/Deny — respond within about a minute, or
it's treated as denied.

`wgaf status` shows which capabilities are restricted and which file the
policy came from. See the README's Configuration section for the file format.

## Putting it together

A typical sequence: focus or find the thing you want, then act on it.

```sh
wgaf window focus 7
wgaf type "hello"

wgaf a11y find --app gtk4-demo --role "push button" --name Save
wgaf a11y click :1.87#/org/a11y/atspi/accessible/1234
```

For a specific error, see the [CLI reference](cli-reference.md)'s error list;
the README also covers the most common setup issue (a stale daemon process
left running). See the [example walkthrough](example-walkthrough.md) for a
full task done start to finish.
