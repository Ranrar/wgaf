# wgaf CLI Reference

Full reference for the `wgaf` command-line tool. For installation and a quick
first run, see the main [README](../README.md).

Every command talks to `wgaf-daemon` over D-Bus — the daemon must already be
running (`wgaf-daemon &`, or as a systemd user service via `make install`).

## Global options

| Flag | Effect |
|---|---|
| `--json` | Emit machine-readable JSON instead of human-readable text. Valid on either side of the subcommand — both `wgaf --json window list` and `wgaf window list --json` work. |
| `--bus-name <NAME>` | Which daemon to talk to, by D-Bus well-known name. Defaults to `org.wgaf.Daemon`. You only need this if the daemon was started with a customised `bus_name` in its `config.toml` — see [Configuration](../README.md#configuration). Also valid on either side of the subcommand. |

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

### `wgaf window resize <id> <width> <height>`

Resizes the window to `width`×`height` pixels, without moving it.

### `wgaf window close <id>`

Closes the window gracefully (same as clicking its close button — not a hard
kill; the app gets a chance to prompt "save changes?").

### `wgaf window workspaces`

Lists all workspaces (`index`, `n_windows`, whether it's the active one).

---

## `wgaf type <text>`

Types a string of text, backed by the daemon's `org.wgaf.Input1` interface via
a virtual `uinput` keyboard device. **ASCII/US-QWERTY only** — there is no
layout/locale awareness. Goes to whatever currently has keyboard focus on the
whole system, not a specific window (use `wgaf window focus` first to target
one).

```sh
wgaf type "hello world"
```

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

---

## `wgaf mouse ...`

Mouse automation, backed by `org.wgaf.Input1`. There is no absolute-move
command — only relative movement, since Wayland has no reliable global
absolute-coordinate authority.

### `wgaf mouse move <dx> <dy>`

Moves the pointer relative to its current position. Either value may be
negative.

**Not pixel-exact.** `libinput` applies pointer acceleration to relative
motion, so the pointer does not necessarily end up exactly `dx`/`dy` pixels
away — a fast large movement travels further than a slow one covering the same
requested distance. Don't rely on it to land on a precise coordinate; prefer
`wgaf a11y` to act on an element directly.

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
- `AT-SPI accessibility bus unavailable` — the accessibility stack isn't
  running for this session.
- `accessible application not found` / `accessible element not found` — the
  `--app` name or element reference doesn't resolve to anything live.
- `invalid element reference` — the element reference isn't well-formed
  `bus_name#object_path` at all (a typo or copy-paste error), as opposed to
  `accessible element not found` above, which means it was well-formed but the
  element it pointed to is gone.
- `action not supported` — the element doesn't implement the action/interface
  you asked for (e.g. `set-text` on a read-only field).
- `permission denied` — the capability is set to `Deny` (or `Prompt` and the
  user declined) in `permissions.toml`. See the "Configuration" section of
  the [README](../README.md) for the policy file format.

Any other failure falls back to the underlying D-Bus error.
