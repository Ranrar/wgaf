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
| `--version`, `-V` | Print this command's version and exit. |
| `--help`, `-h` | Print help. Works on any subcommand: `wgaf workspace switch --help`. |

Commands that don't return data (`window focus`, `type`, `mouse click`, ...)
still respect `--json`: they print `{"ok": true, "message": "..."}` instead of
a plain sentence. `wgaf ping --json` prints `{"ok": true, "response": "pong"}`.

**`--version` is the exception and stays plain text under `--json`**, because
`name x.y.z` is the shape every command-line tool prints and scripts expect. It
also reports the version of *this command*, which need not be the version of the
daemon answering it — a daemon that has been running for a week can be older
than a CLI you just rebuilt. When that distinction matters, `wgaf status --json`
reports the daemon's own version as `daemon_version`.

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

### `wgaf window watch`

Streams window events as they happen, until you stop it with Ctrl-C. One line
per event:

```
created       242
focus-changed 242
closed        242
```

Three things are reported: a window opening, a window closing, and keyboard
focus moving to a window.

**Each event carries the window's id and nothing else.** Run `wgaf window list`
if you need the title or geometry. That is not an oversight: a window has no
title and no size at the instant it is created — the compositor announces it
before the application has drawn anything — so a title reported here would be
blank. Looking it up afterwards also gives you an honest answer when the window
has already gone.

With `--json`, each event is a single line of JSON rather than one big array:

```json
{"event":"created","id":242}
{"event":"focus-changed","id":242}
```

That is deliberate, so it can be piped into something that reads a line at a
time:

```sh
wgaf window watch --json | while read -r line; do
    echo "$line" | jq -r '"\(.event) \(.id)"'
done
```

An array would only be closed when the command ended, so nothing downstream
would see anything until you stopped it — and nothing at all if it were killed.

**There is no replay.** You see what happens from the moment the command starts.
Anything before that is gone and cannot be asked for; `wgaf window list` is the
snapshot, this is the feed.

Needs the GNOME Shell extension installed and enabled, and the `WatchWindows`
permission. If your policy denies it, the command says so and names
`permissions.toml` rather than sitting there showing nothing — on a quiet
desktop those two look identical, which is why it fails out loud.

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

### `wgaf window move-to-workspace <id> <index>`

Sends a window to another workspace.

**The window moves; you do not.** The workspace you are looking at does not
change, so this puts a window out of the way rather than taking you with it.
Follow it with `wgaf workspace switch <index>` to go too.

The command does not return until the window is actually on that workspace, so
the next thing you run sees it there.

The workspace has to exist. An index that does not is refused rather than
created — run `wgaf workspace add` first if you need a new one. Requires the
`MoveWindowToWorkspace` permission, which is separate from the workspace ones:
denying `SwitchWorkspace` and friends does not stop this, and denying this does
not stop them.

```sh
wgaf window move-to-workspace 42 1     # send it away
wgaf workspace switch 1                # and follow it
```

---

## `wgaf workspace ...`

Workspace management, backed by the same `org.wgaf.Windows1` interface (and the
GNOME Shell Extension behind it). Indices are the ones `wgaf workspace list`
reports.

**Every index you have read goes stale as soon as a workspace is added, removed
or reordered.** GNOME numbers workspaces by position, so anything that changes
the order changes what a number means. Re-read the list rather than reusing an
index across such a command.

### `wgaf workspace list`

Lists all workspaces (`index`, `n_windows`, whether it's the active one).

```
  0  windows=6    [active]
  1  windows=0
```

### `wgaf workspace layout`

Shows how the workspaces are arranged:

```
workspaces: 2
active:     0
grid:       2 rows x 1 columns
managed by: GNOME (dynamic workspaces — an added workspace is reclaimed once it is left empty)
```

The last line is the one to read before using `add` or `remove`. GNOME's default
is to manage the number of workspaces for you: it keeps one empty workspace at
the end and takes back any other that empties. If that is on, `wgaf workspace
add` really does add a workspace, and GNOME may well remove it again the moment
nothing is on it. Turn it off with:

```sh
gsettings set org.gnome.mutter dynamic-workspaces false
```

The grid is what "the workspace to the right" means. A standard GNOME setup
reports **one row** with a column per workspace, so on most desktops "to the
right" is simply the next index.

Both numbers are always positive. GNOME's own answer for the column count is
`-1`, meaning "as many as needed" rather than a count; wgaf works out what that
comes to, so `rows × columns` always has room for every workspace and you never
have to handle a negative.

### `wgaf workspace switch <index>`

Switches to a workspace.

The command does not return until that workspace is actually active, so you can
follow it with `wgaf window list` without racing the switch. If the switch never
takes effect the command says so and exits 4 (see [Exit
codes](#exit-codes)) — nothing broke and nothing changed, so it is worth
retrying rather than treating as a failure.

### `wgaf workspace add`

Adds a workspace at the end and prints its index.

It does **not** switch to the new workspace — run `wgaf workspace switch` if you
want that. Read `wgaf workspace layout` first if you are not sure whether GNOME
is managing the count (see above).

### `wgaf workspace remove <index>`

Removes a workspace.

Windows on it are **not** closed: GNOME moves them to a neighbouring workspace,
exactly as it does when you remove one from the overview. The last remaining
workspace cannot be removed, and asking to says so.

### `wgaf workspace reorder <index> <new-index>`

Moves a workspace to a different position. Every other workspace shifts to make
room, so re-read `wgaf workspace list` afterwards.

---

## `wgaf monitor ...`

### `wgaf monitor list`

Lists the monitors making up your desktop.

```
DP-3        2560x1440  at   1080,0       [primary]
            2560x1403  at   1080,37      usable area
HDMI-1      1080x1920  at      0,0       [rotated 90]
```

Positions and sizes are in the same coordinates as `wgaf window list` and
`wgaf mouse move-to`, and are already adjusted for scaling and rotation — so a
point inside one of these rectangles is one the pointer can actually be moved
to. A rotated monitor is reported at its rotated size (the example above is a
1920x1080 panel turned on its side), and a scaled one at its scaled size.

The second line appears only when something is reserving space — GNOME's top
bar, a dock. **That is the rectangle to size a window against**: a window moved
and resized to the full monitor geometry sits partly underneath the top bar.

`--json` adds `connector`, `scale`, `transform` and a `work_area` object per
monitor. `work_area` is `null` when the usable area could not be determined,
which is the case when the wgaf GNOME Shell extension is not installed — the
monitor list itself comes from GNOME directly and does not need it. A `null`
there means "not known", never "nothing is reserving space"; the latter reports
a `work_area` equal to the monitor.

This is the one command that works without the extension.

---

## `wgaf type <text>`

Types a string of text, backed by the daemon's `org.wgaf.Input1` interface via
a virtual `uinput` keyboard device. Uses the keyboard layout your desktop is
set to, so the characters you ask for are the characters that arrive — see
[Keyboard layouts](#keyboard-layouts) below. Without `--window`, goes to
whatever currently has keyboard focus on the whole system.

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

### Targeting a specific window

```sh
wgaf type "hello world" --window 7
```

`--window <id>` types into that window specifically. The daemon resolves the
id, and if that window isn't focused, focuses it first and waits — up to two
seconds — for confirmation that the correction actually took, rather than
assuming it worked and typing anyway. Correcting focus uses the `FocusWindow`
capability, so it is refused if that capability is denied. If the window
can't be confirmed focused in time, nothing is typed and the command fails
saying so.

On a long call, focus is reconfirmed periodically rather than only once at
the start — every 32 characters — so a window losing focus partway through a
long paste stops the rest of it landing in the new window. A call that fails
partway reports how many characters had already gone through.

This is only enforced when `verification_level` in `config.toml` is not
`none` — see [Configuration](configuration.md#action-verification). With
`none`, `--window` is accepted but has no effect, exactly as if it were
omitted; running with no GNOME Shell Extension installed at all still works,
the same as without `--window`.

Omitting `--window` behaves exactly as before it existed.

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

Both accept an optional `--window <id>`, targeting a specific window the same
way `wgaf type --window` does — correcting focus first and confirming it
before pressing anything. See
["Targeting a specific window"](#targeting-a-specific-window) above; the same
`verification_level` dependency applies.

```sh
wgaf key press a --window 7
wgaf key release a --window 7
```

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

Also accepts an optional `--window <id>`, targeting a specific window the
same way `wgaf type --window` does:

```sh
wgaf key combo ctrl shift t --window 7
```

See ["Targeting a specific window"](#targeting-a-specific-window) above; the
same `verification_level` dependency applies.

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
| `--role` | no | The kind of control, case-insensitive, **whole-value match**. The words come from the application's own toolkit, so run `wgaf a11y tree` first and use what it prints — GTK applications say `button` and `text box` where others say `push button` and `entry`. A role that does not match returns nothing rather than an error. Empty (default) matches any role. |
| `--name` | no | Case-insensitive substring match against the element's accessible name. Empty (default) matches any name. |
| `--description` | no | Case-insensitive substring match against the element's accessible description. Empty (default) matches any description. |
| `--max-results` | no | Cap on results. `0` (default) uses the daemon's built-in default of 100; hard-capped at 1000 regardless of what you pass. |

```sh
wgaf a11y find --app "Text Editor" --role button --name Save
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

**This does not work on GTK 4 applications, and that is not something wgaf can
fix.** GTK 4's accessibility bridge refuses the underlying request for every
widget — measured across GTK 4.22, including on buttons that take focus
perfectly well when you press Tab. Since most of the GNOME desktop is GTK, expect
this command to fail more often than not.

Use `wgaf window focus` to focus the *window*, and `wgaf a11y click` to operate
the control you were aiming at. Between them they cover nearly every reason you
would have wanted this.

### `wgaf a11y set-text <element-ref> <text>`

Replaces an element's text content. Requires the element to implement AT-SPI's
`EditableText` interface (most text fields do) — fails with "action not
supported" on elements that don't (e.g. read-only text views).

---

## Error messages

Failures from the daemon are translated into short, specific messages rather
than a raw D-Bus error dump, for example:

- `window not found` — no window with that id (it may have closed). Also
  returned by `wgaf type`/`wgaf key press`/`wgaf key release`/
  `wgaf key combo` when `--window` names an id that doesn't exist, not only
  by `wgaf window` commands.
- `workspace not found` — no workspace at that index. Workspace indices shift
  whenever one is added, removed or reordered, so an index read before such a
  command may well have gone stale; re-read `wgaf workspace list`.
- `the operation was not applied` — the request was allowed and attempted, and
  the desktop did not change. Nothing broke and nothing refused, so this is
  worth retrying — see exit code `4` below. Also how "the last workspace cannot
  be removed" is reported.
- `GNOME Shell Extension bridge unavailable` — the extension isn't
  installed/enabled, or the daemon can't reach it. If the message says the
  interface "has no `<Name>` method", the extension is older than the daemon:
  reinstall it and log out and back in.
- `monitor layout unavailable` — GNOME's display configuration couldn't be read,
  so `wgaf monitor list` has nothing to report. Distinct from the extension
  being unavailable: this comes from GNOME itself, so the usual cause is not
  being on a GNOME session at all.
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
- `focus could not be verified` — `--window` named a window that couldn't be
  confirmed focused in time (most often GNOME's own focus-stealing
  prevention declining the correction). Nothing was typed; if the failure
  happened partway through a long `wgaf type`, the message says how many
  characters had already gone through. See
  ["Targeting a specific window"](#targeting-a-specific-window) above.
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

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Success. |
| `1` | Error — something failed unexpectedly. Printed with an `error:` prefix. |
| `2` | Not from wgaf: clap's own exit code for a malformed command line (an unknown flag, a missing argument). The daemon never sees the request. |
| `3` | Denied — refused by `permissions.toml` policy, the kill switch (`wgaf stop` or Escape), or a configured limit (`text too long`, `input rate limit exceeded`). |
| `4` | Unverified — the command was allowed and attempted and the desktop is not what you assumed. A `--window` target's focus could not be confirmed (`focus could not be verified`), or a workspace command's change never took effect (`the operation was not applied`). Worth retrying. |

Only exit code `1` prints the `error:` prefix. `3` and `4` print the daemon's
own message with no prefix — it already reads as a complete sentence, not a
malfunction. `--json` mode adds an `"outcome"` field to a failure instead:
`"error"`, `"denied"`, or `"unverified"`.
