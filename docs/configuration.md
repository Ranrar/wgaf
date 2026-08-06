# Configuration

wgaf keeps its settings in two TOML files in `~/.config/wgaf/` (or
`$XDG_CONFIG_HOME/wgaf/` if set), which the daemon finds on its own:

| File | Purpose |
|---|---|
| `config.toml` | Bus name, log level, device name, keyboard layout, input safety limits, action verification, device settle time |
| `permissions.toml` | What wgaf is allowed to do |

`make install` sets both up for you, with the right ownership and mode, and
never overwrites files you already have. The templates it installs spell out
every setting and capability at its default value, so you can see and edit the
whole surface in place — they change nothing until you edit something. Comment
a line out and it goes back to tracking the built-in default.

With plain `cargo install`, create them once:

```sh
mkdir -p ~/.config/wgaf
: > ~/.config/wgaf/config.toml
printf '[capabilities]\n' > ~/.config/wgaf/permissions.toml
chmod 600 ~/.config/wgaf/config.toml ~/.config/wgaf/permissions.toml
```

Empty files select the defaults: an empty `config.toml` uses the built-in
settings, and an empty `[capabilities]` table allows every capability. Keep
both readable and writable by you alone (mode `600`) — wgaf only runs on
configuration it can tell is yours, and will say so if something needs
fixing.

`wgaf status` shows which files are in use, what's restricted, and — if a file
is missing — where it should go.

## Using different paths, or no policy file at all

```sh
wgaf-daemon --config /path/to/config.toml --permissions /path/to/permissions.toml
```

`--permissions` defaults to a `permissions.toml` next to whichever
`config.toml` was resolved, so moving `--config` moves both.

`--config-optional` and `--permissions-optional` skip the respective file and
use the built-in defaults (for the policy, that means allowing everything, and
it logs a warning). The empty files above are preferable — they say the same
thing but stay visible in your config.

The full list of daemon flags is in the
[CLI reference](cli-reference.md#daemon-configuration).

## Keyboard layout

`wgaf type` uses the layout your desktop is set to. `input_keyboard_layout`
overrides that:

| Value | Meaning |
|---|---|
| `"auto"` | Your session's layout. The default. |
| `"dk"` | A layout code — `localectl list-x11-keymap-layouts` |
| `"dk(nodeadkeys)"` | A code with a variant — `localectl list-x11-keymap-variants dk` |
| `"Danish"`, `"English (Dvorak)"` | The layout's full name |
| `"us-ascii"` | Ignore your layout; use a plain US keyboard |

A layout, not a language: `"en"` is refused, since English has ten of them. An
unknown layout stops the daemon rather than falling back to another one.

Read once at startup — after changing your layout, restart the daemon
(`systemctl --user restart wgaf-daemon.service`). `wgaf status` shows the one in
use.

## Input safety limits

Two settings in `config.toml` bound how much synthetic input wgaf will produce:

| Setting | Default | What it does |
|---|---|---|
| `input_max_events_per_second` | `3000` | Sustained keystrokes and clicks per second. Going over **slows commands down rather than failing them**, so a long automation still finishes. `0` turns the limit off. |
| `input_max_type_text_chars` | `4096` | Most characters one `wgaf type` may send. Longer text is **refused outright** — nothing is typed. Careful: `0` here means nothing may be typed, *not* "no limit". |

Both exist for one situation: a script with a loop bug, or a paste far longer
than you meant. Without them the flood competes with your own keyboard and
mouse, and taking back control of the desktop is genuinely hard.

The defaults are generous — far beyond anything normal automation needs — so
you are unlikely to meet either by accident. Lower them if you want a tighter
guard.

The [user guide](user-guide.md#if-automation-suddenly-runs-slowly) covers both
in more detail, including what it looks like when you do meet one.

Neither is a substitute for the emergency stop: press **Escape** to stop input
automation outright, and `wgaf release` to allow it again. Escape is only taken
from your applications while a run is in progress — see the
[user guide](user-guide.md#escape-is-only-borrowed-while-automation-runs).

## Action verification

`--window <id>` on `wgaf type`/`wgaf key press`/`wgaf key release`/
`wgaf key combo` (see the [CLI reference](cli-reference.md#targeting-a-specific-window))
asks the daemon to confirm the named window is focused — correcting it first
if it isn't — before sending anything. `verification_level` controls how much
of that actually happens:

| Value | Meaning |
|---|---|
| `"none"` | `--window` is accepted but never checked. Exactly the behaviour from before this setting existed. |
| `"basic"` | The default. A named target that can't be confirmed focused fails the call rather than being typed into anyway. |
| `"strict"` | Reserved for an AT-SPI-based post-check that doesn't exist yet. Naming it is rejected at daemon startup, not silently run as `"basic"` — the daemon won't claim to perform a check it doesn't actually have. |

`basic` being the default is a deliberate choice: a `--window` target is
guarded unless you turn checking off, rather than only for scripts that know
to opt into `verification_level` themselves.

## `permissions.toml` — per-capability policy

Nineteen capabilities exist, one per gated command. Read-only commands
(`window list`, `workspace list`, `monitor list`, `a11y find`, etc.) can't be
gated at all:

| Interface | Capabilities |
|---|---|
| `org.wgaf.Windows1` — windows | `FocusWindow`, `MoveWindow`, `ResizeWindow`, `CloseWindow`, `MoveWindowToWorkspace`, `WatchWindows` |
| `org.wgaf.Windows1` — workspaces | `SwitchWorkspace`, `AddWorkspace`, `RemoveWorkspace`, `ReorderWorkspace` |
| `org.wgaf.Input1` | `TypeText`, `KeyPress`, `KeyRelease`, `MouseMove`, `MouseMoveAbsolute`, `MouseClick`, `MouseScroll` |
| `org.wgaf.Accessibility1` | `InvokeAction`, `SetText`, `FocusElement` |

Each name is the D-Bus method it gates, and the command it belongs to is the
obvious one — `SwitchWorkspace` is `wgaf workspace switch`, and so on.

`WatchWindows` is the odd one: it gates *observing* rather than acting, since
`wgaf window watch` opens a feed of every window title that appears for as long
as it runs. Note it is a statement of intent rather than a wall — anything
running as you can listen on the session bus without asking wgaf — but denying
it means `wgaf window watch` says so instead of sitting on a silent stream.

The four workspace capabilities are deliberately separate rather than one
`ManageWorkspaces`, so that "let automation move around my desktop, but not
rearrange it" is a thing you can actually write:

```toml
[capabilities]
AddWorkspace = "Deny"
RemoveWorkspace = "Deny"
ReorderWorkspace = "Deny"
# SwitchWorkspace stays allowed
```

`MoveWindowToWorkspace` is listed with the *window* capabilities above rather
than with these, and gated separately from both. It changes neither how many
workspaces exist nor which one you are on — it takes one of your windows out of
sight. Allowing automation to navigate your desktop is not the same as letting
it rearrange your windows behind you, so denying it leaves `wgaf workspace
switch` working and stops `wgaf window move-to-workspace`.

Each can be `Allow`, `Deny`, or `Prompt`:

```toml
# ~/.config/wgaf/permissions.toml
[capabilities]
TypeText = "Deny"         # block `wgaf type` entirely
CloseWindow = "Prompt"    # ask via a GNOME notification (Allow/Deny) before closing a window
```

With the above, `wgaf type "secret"` is refused outright, and
`wgaf window close 7` raises a desktop notification and waits for your
answer.

Any capability not listed defaults to `Allow` — this is a personal automation
tool, so permissions are an opt-in *restriction* you configure, never an
*unlock* you must grant before anything works. Only the file itself is
mandatory; what you put in it is up to you, and an empty `[capabilities]`
table restricts nothing.

Every command that changes something passes this policy before it runs, and is
recorded afterwards, whether it came from you at a terminal or from a script.

---

[Installation](installation.md) · [User guide](user-guide.md) ·
[CLI reference](cli-reference.md) · [Example walkthrough](example-walkthrough.md)
