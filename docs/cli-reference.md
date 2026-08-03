# wgaf CLI Reference

Full reference for the `wgaf` command-line tool. For installation and a quick
first run, see the main [README](../README.md).

Every command talks to `wgaf-daemon` over D-Bus — the daemon must already be
running (`wgaf-daemon &`, or as a systemd user service via `make install`).

## Global options

| Flag | Effect |
|---|---|
| `--json` | Emit machine-readable JSON instead of human-readable text. Valid on either side of the subcommand — both `wgaf --json window list` and `wgaf window list --json` work. |
| `--bus-name <NAME>` | Which daemon to talk to, by D-Bus well-known name. Defaults to `org.wgaf.Daemon`. You only need this if the daemon was started with a customised `bus_name` in its `config.toml` — see [Configuration](configuration.md). Also valid on either side of the subcommand. |

Commands that don't return data (`window focus`, `type`, `mouse click`, ...)
still respect `--json`: they print `{"ok": true, "message": "..."}` instead of
a plain sentence. `wgaf ping --json` prints `{"ok": true, "response": "pong"}`.

Shell completions: `wgaf completions <bash|zsh|fish|elvish|powershell>` prints
a completion script to stdout — see the main README for how to install it for
your shell.

## Daemon configuration

`wgaf-daemon` finds its configuration on its own — no flags required. It looks
for `config.toml` in `$XDG_CONFIG_HOME/wgaf/` (falling back to
`~/.config/wgaf/` when `$XDG_CONFIG_HOME` is unset or empty), and for
`permissions.toml` as a sibling of whichever `config.toml` it resolved.

Both files are expected to be present, owned by the user the daemon runs as,
and mode `600`; the daemon reports what to fix on startup if not. Empty files
select the defaults — for the policy, a file containing just `[capabilities]`.

Either path can be pointed elsewhere when starting the daemon:

| Daemon flag | Effect |
|---|---|
| `--config <PATH>` | Use this `config.toml` instead of the resolved default. |
| `--permissions <PATH>` | Use this policy file instead of the `config.toml` sibling. |
| `--config-optional` | Run with no config file, using built-in defaults. Prefer an empty file, which states the same thing but leaves a record. |
| `--permissions-optional` | Run with no policy file, allowing every capability. Logs a warning. Prefer an empty `[capabilities]` file. |
| `--log-level <LEVEL>` | Override `config.toml`'s `log_level` for this run. |

`wgaf status` reports which files are actually in effect — see below. `make
install` puts a template of each in place without overwriting anything you
already have.

---

## `wgaf ping`

Checks that the daemon is running and responding. Prints `pong` (or the JSON
form above).

```sh
wgaf ping
```

---

## `wgaf status`

Reports whether each subsystem is set up correctly, and what permission policy
the daemon is enforcing. Where `wgaf ping` only proves the daemon answers,
`status` checks the three things that actually have to be configured — the
GNOME Shell Extension bridge, `/dev/uinput` access, and the AT-SPI
accessibility bus — and prints the daemon's own guidance for whichever is not
working.

Start here when something isn't behaving, and include its output in bug
reports.

```sh
wgaf status
```

```text
wgaf 0.7.0 — pid 34804, up 1s, on org.wgaf.Daemon
config: /home/you/.config/wgaf/config.toml

[fail] GNOME Shell Extension  (org.gnome.Shell.Extensions.Wgaf)
       GNOME Shell Extension bridge unavailable: no owner for the extension's
       D-Bus name — the wgaf GNOME Shell Extension is not installed or not
       enabled ...
[ ok ] Input (/dev/uinput)    (wgaf virtual input device, no device created yet)
[ ok ] Accessibility (AT-SPI) (not connected yet)

permissions: /home/you/.config/wgaf/permissions.toml
       no capability restricted — every capability allowed
```

**Exit code** is `0` when every subsystem is available and `1` when any is
unavailable, so it can gate a setup script:

```sh
if wgaf status >/dev/null; then echo "ready"; fi
```

Notes on reading the output:

- *"no device created yet"* and *"not connected yet"* are **not** problems.
  They report whether the daemon currently holds a virtual input device or an
  open accessibility connection, which it creates lazily on first use. They are
  activity indicators, not health ones — the `[ ok ]`/`[fail]` marker is what
  tells you whether the subsystem works.
- Running `wgaf status` never creates the virtual input device, opens a cached
  connection, or changes any state. It only reports.
- Config and permissions paths are shown whether or not the files exist, with
  absent ones marked — so this doubles as the answer to "where do those files
  go?". You will only see one marked absent if the daemon was started with
  `--config-optional` or `--permissions-optional`.
- Only the `--json` output is a stable, parseable interface; the human-readable
  layout may change between versions.
- If the kill switch is engaged, the report opens with `!! INPUT STOPPED`
  before anything else. That is the answer to "why is nothing happening"; run
  `wgaf release` to allow input again.

---

## `wgaf stop`

Stops all input automation immediately. Use it when a script has run away with
your keyboard or pointer. Pressing **Escape** does the same thing without a
terminal, provided the GNOME Shell Extension is installed and enabled.

```sh
wgaf stop
```

- A command already in progress is aborted part-way.
- Every later `type`, `key` or `mouse` command is refused.
- The daemon's virtual input device is removed.
- Window, workspace and accessibility commands are unaffected.
- No permission policy can take it away, and it is never saved to disk.

---

## `wgaf release`

Releases the emergency stop, allowing input automation again.

```sh
wgaf release
```

It does not resume what was stopped — run your command again if you still want
it. Restarting the daemon has the same effect. See the
[user guide](user-guide.md#emergency-stop--pulling-the-handbrake) for why the
release is manual.

---

## `wgaf window ...`

Window management, backed by the daemon's `org.wgaf.Windows1` interface (and,
behind that, the GNOME Shell Extension). All ids are the value reported by
`wgaf window list`, not a window manager ID from any other tool.

### `wgaf window list`

Lists all windows. Human-readable output is one line per window:

```
   7  ws=0     240,76     800x600   org.gtk.Demo4   Assistant  [focused, maximized]
```

`--json` prints the full array of window records (`id`, `title`, `app_id`,
`workspace`, `x`, `y`, `width`, `height`, `focused`, `maximized`).

Transient surfaces — tooltips, open menus, combo-box dropdowns — are
deliberately left out, since they aren't things you'd sensibly focus, move,
resize, or close. If a menu is open when you run this, it won't be listed.

### `wgaf window focus <id>`

Focuses (activates) the window with the given id.

### `wgaf window move <id> <x> <y>`

Moves the window so its top-left corner lands at `(x, y)`. `x`/`y` may be
negative (e.g. a monitor positioned left of or above the primary one).

**The move happens in one step**, with no intermediate positions and no
animation — the window is at its old position, then its new one. The
application is told once where it ended up; it does not see a sequence of
positions along the way, so anything watching for a drag-like series of moves
will not see one. `wgaf window resize` and `wgaf mouse move-to` work the same
way.

### `wgaf window resize <id> <width> <height>`

Resizes the window to `width`×`height` pixels, without moving it.

Applied in one step, exactly as `wgaf window move` describes: the window jumps
from the old size to the new one, and the application is told once rather than
being asked to relayout repeatedly.

**The command returns before the new size can be read back.** For a moment
afterwards — around 30 ms on a typical desktop — `wgaf window list` still
reports the *old* rectangle. This matters if you compute a coordinate from it:

```sh
# Wrong: the list may still describe the old size, so this clicks the old centre
wgaf window resize "$id" 720 560
wgaf mouse move-to "$new_centre_x" "$new_centre_y"
```

Nothing errors — the pointer goes exactly where you sent it, which just is not
where you meant. If you need the new geometry, poll `wgaf window list` until it
reports the size you asked for before using it. The same caution applies to
`wgaf window move`.

### `wgaf window close <id>`

Closes the window gracefully (same as clicking its close button — not a hard
kill; the app gets a chance to prompt "save changes?").

### `wgaf window workspaces`

Lists all workspaces (`index`, `n_windows`, whether it's the active one).

---

## `wgaf type <text>`

Types a string of text, backed by the daemon's `org.wgaf.Input1` interface via
a virtual `uinput` keyboard device. Uses the keyboard layout your desktop is
set to, so the characters you ask for are the characters that arrive — see
[Keyboard layouts](#keyboard-layouts) below. Goes to whatever currently has
keyboard focus on the whole system, not a specific window (use
`wgaf window focus` first to target one).

```sh
wgaf type "hello world"
```

Capped at 4096 characters per command by default. Longer text is refused
outright — nothing is typed — with a message naming the limit. Change it with
`input_max_type_text_chars` in `config.toml`; lower it if you would rather an
oversized paste fail than be typed into whichever window has focus. See
["Limiting how much one command can type"](user-guide.md#limiting-how-much-one-command-can-type).

Typing is also subject to the overall input speed limit, which paces commands
against each other rather than capping any one of them. See ["If automation
suddenly runs slowly"](user-guide.md#if-automation-suddenly-runs-slowly).

### Keyboard layouts

`wgaf type` uses the keyboard layout your desktop is set to, so text arrives as
you wrote it whatever that layout is — including characters that need AltGr,
like `@` and `{` on a Danish keyboard, and characters behind a dead key, like
`~`.

You do not normally need to configure anything. If you want to pin a specific
layout, set `input_keyboard_layout` in `config.toml` to a layout code (`dk`), a
code with a variant (`dk(nodeadkeys)`), or its full name (`Danish`,
`English (Dvorak)`). `localectl list-x11-keymap-layouts` lists the codes.

Three things are worth knowing:

- **The layout is read once, when the daemon starts.** Change your keyboard
  layout afterwards and you will need to restart the daemon:
  `systemctl --user restart wgaf-daemon.service`. `wgaf status` shows which
  layout is in use.
- **A character your layout cannot produce is refused, not approximated.**
  Asking for an emoji fails with a message naming the character, and nothing is
  typed at all — not even the part of the text before it.
- **This is a layout, not a language.** `en` is not a valid setting: English has
  ten layouts, and a US and a Dvorak keyboard put nearly every key somewhere
  different.

---

## `wgaf key press|release <key>`

Low-level single-key press/release, by evdev key name (`a`, `KEY_A`, `enter`,
`leftshift`, ...) — no ASCII/shift awareness. Use `wgaf type` for typing
actual text; use `key` when you need to hold a modifier across multiple keys,
e.g. a capital `A`:

```sh
wgaf key press leftshift
wgaf key press a
wgaf key release a
wgaf key release leftshift
```

Every key press must be matched by a release. A key left pressed stays pressed
for the rest of the session, exactly as a physically stuck key would.

**`escape` does not reach applications while wgaf is running.** Escape is the
emergency stop, so the desktop hands it to wgaf rather than to whatever you are
automating. wgaf recognizes its own keystrokes and will not stop itself on one,
but the key does not arrive at the application either — so pressing Escape at a
dialog will not close it. Use the [`wgaf a11y`](#wgaf-a11y-) commands to press
the dialog's own Cancel or Close button, which is more dependable than a
keystroke in any case.

This applies only to whichever key is set as the emergency stop; every other
key is unaffected.

---

## `wgaf key combo <key>...`

Press a key combination — every key held down in order, then released in
reverse:

```sh
wgaf key combo ctrl shift t
```

Prefer this over the four-command form above whenever you want a shortcut. It
is not just shorter: if one of the separate commands fails, the modifiers stay
held down and the session then behaves as though you are leaning on Ctrl, with
nothing on screen to say why. `combo` presses nothing at all unless every key
name is valid.

Combinations are physical keys rather than characters, so the same command
works on every keyboard layout — unlike `wgaf type`.

### Key names

Names are case-insensitive and the `KEY_` prefix is optional, so `a`, `A`,
`KEY_A` and `key_a` are the same key.

| Group | Names |
|---|---|
| Letters and digits | `a`–`z`, `0`–`9` |
| Punctuation | `minus` (`dash`), `equal`, `leftbrace`, `rightbrace`, `semicolon`, `apostrophe` (`quote`), `grave` (`backtick`), `backslash`, `comma`, `dot` (`period`), `slash` |
| Editing | `enter` (`return`), `tab`, `space`, `backspace`, `delete` (`del`), `insert` (`ins`), `escape` (`esc`) |
| Arrows and navigation | `up`, `down`, `left`, `right`, `home`, `end`, `pageup` (`pgup`), `pagedown` (`pgdn`) |
| Function keys | `f1`–`f12` |
| Modifiers | `leftshift` (`shift`), `rightshift`, `leftctrl` (`ctrl`), `rightctrl`, `leftalt` (`alt`), `rightalt` (`altgr`), `leftmeta` (`super`, `win`), `rightmeta`, `capslock` (`caps`) |
| Keypad | `kp0`–`kp9`, `kpdot`, `kpplus`, `kpminus`, `kpasterisk`, `kpslash`, `kpenter`, `numlock` |
| Other | `102nd` (the extra `<>` or `\|` key on ISO keyboards), `printscreen`, `scrolllock`, `pause`, `menu` |

An unrecognised name is refused by name rather than ignored.

### Keys and your keyboard layout

A key name refers to a **physical key position**, not to the character printed
on it, and the names follow a US keyboard. On another layout a key produces
whatever that position produces there: `wgaf key press 2` with shift held gives
`"` on a Danish keyboard, not `@`.

That is deliberate: shortcuts are positions, so they work the same everywhere.
For a character rather than a key, use `wgaf type` — it resolves against your
layout and would give you the `@`.

---

## `wgaf mouse ...`

Mouse automation, backed by `org.wgaf.Input1`.

Two ways to move the pointer, and they behave differently: `move` is relative
and approximate, `move-to` is absolute and exact. Prefer `wgaf a11y` over either
when an element can be found by name or role.

### `wgaf mouse move <dx> <dy>`

Moves the pointer relative to its current position. Either value may be
negative.

**Not pixel-exact.** `libinput` applies pointer acceleration to relative
motion, so the pointer does not necessarily end up exactly `dx`/`dy` pixels
away — a fast large movement travels further than a slow one covering the same
requested distance. Don't rely on it to land on a precise coordinate; use
`wgaf mouse move-to`, or prefer `wgaf a11y` to act on an element directly.

### `wgaf mouse move-to <x> <y>`

Moves the pointer to an absolute position, in screen pixels measured from the
top-left of your desktop layout. Prints the position the pointer ended up at.

Either value may be negative — a monitor placed left of or above your primary
one has negative coordinates.

**This one is pixel-exact.** Unlike `wgaf mouse move`, no pointer acceleration
is involved: the pointer lands on exactly the coordinate you asked for.

**The move happens in one step**, with no intermediate positions — the same
contract as `wgaf window move` and `wgaf window resize`. An application sees the
pointer arrive, along with the usual enter/leave as it crosses windows, but it
never sees it travel: anything watching for a drag-like sequence of movements
will not see one.

**A position that is not on a monitor is refused, and nothing moves.** Worth
knowing if you compute coordinates: a desktop whose monitors differ in size or
alignment has gaps. With a tall monitor beside a short one, for instance, part
of the overall rectangle is not on any screen, and a coordinate there is
rejected rather than nudged to the nearest visible pixel. The error lists your
monitors and their positions.

```console
$ wgaf mouse move-to 1500 700
moved pointer to (1500, 700)

$ wgaf mouse move-to 2000 1700
error: off screen: (2000, 1700) is not on any monitor — the current layout is:
HDMI-1 1080x1920 at (0,0), DP-3 2560x1440 at (1080,0)
```

### `wgaf mouse position`

Prints the pointer's current position as `x y`, or as a JSON object with `--json`.

```console
$ wgaf mouse position
1500 700
```

### `wgaf mouse click <button>`

Clicks (press then release) a mouse button: `left`, `right`, or `middle`.

### `wgaf mouse scroll <dx> <dy>`

Scrolls the wheel. `dx` is horizontal (positive = right), `dy` is vertical
(positive = up). Either may be negative.

---

## `wgaf a11y ...`

Accessibility automation via AT-SPI, backed by `org.wgaf.Accessibility1`.
Preferred over window/mouse coordinate automation whenever an element can be
found this way — it's robust to window resizing/theme changes and doesn't
need coordinates at all.

Elements are referenced as `bus_name#object_path` (AT-SPI's own native,
stable reference — e.g. `:1.87#/org/a11y/atspi/accessible/1234`), as printed
by `list-apps`/`find`/`tree`.

### `wgaf a11y list-apps`

Lists every currently-registered accessible application (name + root element
reference).

### `wgaf a11y find --app <name> [--role <role>] [--name <name>] [--description <desc>] [--max-results <n>]`

Finds elements within an application.

| Flag | Required | Meaning |
|---|---|---|
| `--app` | yes | Application name — matched against `list-apps`' output; exact match preferred, falls back to substring. |
| `--role` | no | AT-SPI role name (e.g. `push button`, `menu item`), case-insensitive, whole-value match. Empty (default) matches any role. |
| `--name` | no | Case-insensitive substring match against the element's accessible name. Empty (default) matches any name. |
| `--description` | no | Case-insensitive substring match against the element's accessible description. Empty (default) matches any description. |
| `--max-results` | no | Cap on results. `0` (default) uses the daemon's built-in default of 100; hard-capped at 1000 regardless of what you pass. |

```sh
wgaf a11y find --app gtk4-demo --role "push button" --name Save
```

### `wgaf a11y tree --app <name> [--max-depth <n>]`

Walks and prints an application's accessible object tree, indented by depth.
`--max-depth` (default `0`, meaning the daemon's built-in default of 10) is
hard-capped at 64 regardless of what you pass.

### `wgaf a11y info <element-ref>`

Prints one element's current info (name, role, description, child count,
states, its own reference), re-read live from the reference.

### `wgaf a11y click <element-ref> [--action <name>]`

Invokes an accessible action on an element (the AT-SPI `Action` interface —
covers "click", "press", "activate", etc.). `--action` selects which one by
its machine-readable name, case-insensitive; omitted (default), it invokes
the element's own default action (AT-SPI action index 0).

### `wgaf a11y focus <element-ref>`

Requests keyboard focus for an element.

### `wgaf a11y set-text <element-ref> <text>`

Replaces an element's text content. Requires the element to implement AT-SPI's
`EditableText` interface (most text fields do) — fails with "action not
supported" on elements that don't (e.g. read-only text views).

---

## Error messages

Failures from the daemon are translated into short, specific messages rather
than a raw D-Bus error dump, for example:

- `window not found` — no window with that id (it may have closed).
- `GNOME Shell Extension bridge unavailable` — the extension isn't
  installed/enabled, or the daemon can't reach it.
- `input device unavailable` — `/dev/uinput` isn't accessible (permissions).
- `unknown key` / `invalid mouse button` — bad argument to `key`/`mouse click`.
- `text too long` — the text given to `wgaf type` is over
  `input_max_type_text_chars`. Nothing was typed. The message names the limit
  in force.
- `input is stopped` — the kill switch is engaged (`wgaf stop`, or
  Escape). Nothing will be synthesized until you run `wgaf release`.
  This is a live emergency stop rather than a policy decision, which is why it
  is not `permission denied`.
- `input rate limit exceeded` — so much synthetic input is queued that this
  command would have waited more than half a minute, which means something is
  stuck in a loop. Note that merely going over the speed limit does *not*
  produce an error; it slows commands down instead.
- `AT-SPI accessibility bus unavailable` — the accessibility bus couldn't be
  reached. The message says which of the possible causes applies — the
  accessibility service isn't available at all, or it's still advertising a bus
  that has since exited — and what to do about it, since the remedies differ.
- `accessible application not found` / `accessible element not found` — the
  `--app` name or element reference doesn't resolve to anything live.
- `invalid element reference` — the element reference isn't well-formed
  `bus_name#object_path` at all (a typo or copy-paste error), as opposed to
  `accessible element not found` above, which means it was well-formed but the
  element it pointed to is gone.
- `action not supported` — the element doesn't implement the action/interface
  you asked for (e.g. `set-text` on a read-only field).
- `permission denied` — the capability is set to `Deny` (or `Prompt` and the
  user declined) in `permissions.toml`. See
  [Configuration](configuration.md#permissionstoml--per-capability-policy) for
  the policy file format.

Any other failure falls back to the underlying D-Bus error.
