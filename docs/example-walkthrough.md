# Example Walkthrough: A to Z

A complete, real-world example of arranging a window, typing a note into
it, and saving it by clicking its Save button — rather than clicking a
fixed screen position. For individual commands, see the
[user guide](user-guide.md) and [CLI reference](cli-reference.md). This
page just ties everything together into one task, with more explanation
than either of those, in case you haven't memorized every GNOME/AT-SPI/D-Bus
detail yet.

**The scenario:** you've just sat down to work. You want your text editor
open, positioned where you like it, and a quick note typed in — all done
automatically instead of by hand.

## Before you start

Open a terminal and check everything this guide needs:

```sh
wgaf status
```

You want the GNOME Shell Extension line to read `[ ok ]` — this walkthrough
moves and resizes windows, which goes through the extension. Input and
Accessibility should be `[ ok ]` too. Any line marked `[fail]` comes with the
fix printed underneath it.

If `wgaf status` can't reach the daemon at all, start it:

```sh
wgaf-daemon &
```

The trailing `&` runs it in the background, so your terminal stays free for
`wgaf` commands. If you installed via `make install` and enabled the systemd
service, it's probably already running.

Should the daemon exit immediately instead, it will say what it wants — most
likely its two config files, which `make install` creates for you and
`cargo install` does not. [Configuration](configuration.md) has the two
commands to create them.

The rest of this guide assumes the daemon and the extension are both working.

## Step 1 — Open the app

wgaf controls windows and apps that are already running — it doesn't
launch them for you. Open your editor the normal way: click it in the
Activities overview, or launch it from the same terminal:

```sh
gnome-text-editor &
```

Any app works the same way for the rest of this walkthrough, so feel free
to swap in whatever you actually want to automate. If it's already open,
skip this step.

## Step 2 — Find the window and note its ID

```sh
wgaf window list
```

```
   9  ws=0     0,0     1000x700   org.gnome.TextEditor   New Document  [focused]
```

Reading left to right: `9` is the window's **ID** — what every other
window command needs. `ws=0` is its workspace. `0,0` is its current
top-left position, and `1000x700` is its current size. Then comes the
application ID and window title, followed by status flags like
`[focused]` or `[maximized]` when they apply.

**The ID only lasts for this session** — it changes if the window closes
and reopens, and yours will almost certainly be a different number than
the `9` shown here. Use whatever number `list` actually gives you in the
commands below, not the literal `9` from this example.

## Step 3 — Position the window

Screen coordinates start at `(0, 0)` in the **top-left corner**: x
increases to the right, y increases downward. So the commands below move
the window 100 pixels from the left and 100 from the top, then resize it
to 900×600 pixels:

```sh
wgaf window focus 9
wgaf window move 9 100 100
wgaf window resize 9 900 600
```

Always focus the window first — the *next* step (typing) goes to whatever
window currently has keyboard focus, not to a window you merely refer to
by ID. You should see the window jump to its new position and size right
away; that's your confirmation it worked. To check programmatically
instead, run:

```sh
wgaf window list
```

and look for the same ID with the updated `x,y` and size.

## Step 4 — Type into it

```sh
wgaf type "Automated with wgaf."
```

You should see the text appear in the editor, since it currently has
focus. One shell-quoting trap to watch for: `wgaf type` does treat `\n`
(newline) and `\t` (tab) as real key presses — Enter and Tab — but only if
those literal bytes actually reach it. Typing `"...\n"` in an ordinary
double-quoted shell argument does **not** produce a real newline byte; it
sends the two characters `\` and `n` instead. If you want to press Enter,
do it as its own step to avoid the trap entirely:

```sh
wgaf key press enter
wgaf key release enter
```

## Step 5 — Find the Save button by name, not position

This step uses **AT-SPI** — the same system screen readers rely on — to
find UI elements by what they *are* (a button named "Save") instead of
guessing their pixel position. That means it keeps working even if the
window ends up a different size than in this example.

First, confirm the app is visible to AT-SPI at all:

```sh
wgaf a11y list-apps
```

Look for your editor's name in the list (e.g. `Text Editor`). If it's not
there, the app may not have finished starting yet, or accessibility
support isn't active for this session — see the
[user guide](user-guide.md) for more.

Next, browse what's actually inside the app. You don't need to guess
names — this shows you everything:

```sh
wgaf a11y tree --app "Text Editor"
```

This prints one line per UI element, each with a **role** (what kind of
thing it is — `push button`, `menu item`, `text`, etc.) and a **name**
(its visible label, roughly). Look for a `push button` named `Save`.
Exact roles and labels vary between apps, and even between versions of the
same app — which is exactly why you check here first instead of assuming.
Once you've found it, narrow down to it directly:

```sh
wgaf a11y find --app "Text Editor" --role "push button" --name Save
```

```
push button          Save                      :1.87#/org/a11y/atspi/accessible/1234
```

That long `:1.87#/org/a11y/atspi/accessible/1234` value is the **element
reference** — a live pointer to that specific on-screen element. Copy the
one your own terminal printed, not the one shown here (yours will differ).
It stops being valid once the element goes away — for example, when the
window closes.

## Step 6 — Click it

```sh
wgaf a11y click :1.87#/org/a11y/atspi/accessible/1234
```

(Using the reference `find` printed for you in the previous step.)

If this fails with `action not supported`, the element you found doesn't
have anything to trigger the way you asked. Double-check that you copied
the exact reference from your own `find` output — not the placeholder
above — and that `--role`/`--name` in Step 5 actually matched the button
you meant.

If the app prompts for a filename on first save, that dialog is a normal
window in its own right. Repeat Steps 2–6 against it if you want to
automate through it too, rather than treating it as a special case.

## If something goes wrong

- **The script is typing where it shouldn't** — press **Escape**. All input
  automation stops immediately, and `wgaf release` allows it again.

- **`GNOME Shell Extension bridge unavailable`** — the extension isn't
  enabled, or hasn't loaded yet. Recheck `gnome-extensions info
  wgaf@wgaf.dev` from "Before you start."
- **`permission denied`** — a `permissions.toml` on your system has that
  action set to `Deny` (or you didn't respond to a `Prompt` notification
  in time). See the README's Configuration section.
- **`accessible application not found`** — the `--app` name didn't match
  anything in `a11y list-apps`'s output. Re-check the exact name there.
- **`action not supported`** — see Step 6 above.

The full error list is in the [CLI reference](cli-reference.md).

## Putting it all together as a script

Once you've confirmed the steps work interactively, the whole thing
becomes a simple shell script. This uses `jq` to pull values out of
`--json` output — install it first if you don't have it
(`sudo apt install jq` on Debian/Ubuntu, `sudo dnf install jq` on Fedora,
or your distro's equivalent).

```sh
#!/bin/sh
set -e

id=$(wgaf --json window list | jq -r '.[] | select(.app_id == "org.gnome.TextEditor") | .id' | head -1)
wgaf window focus "$id"
wgaf window move "$id" 100 100
wgaf window resize "$id" 900 600
wgaf type "Automated with wgaf."

ref=$(wgaf --json a11y find --app "Text Editor" --role "push button" --name Save \
    | jq -r '.[0].element | "\(.bus_name)#\(.object_path)"')
wgaf a11y click "$ref"
```

`set -e` stops the script at the first failing command instead of letting
it plow on — useful while you're still testing. Save this to a file (e.g.
`automate.sh`), make it executable once, and run it any time:

```sh
chmod +x automate.sh
./automate.sh
```

This is the general pattern for any task: query for the current ID or
element reference with `--json` piped through `jq`, then act on what you
got. Never hardcode an ID or element reference across runs — both are only
valid for the current session.