# Test applications

GTK4 applications that wgaf drives in its tests, and that report what they
actually observed.

Build them with `make test-apps` from the repository root. They need GTK4
development packages (`libgtk-4-dev` on Debian/Ubuntu, `gtk4-devel` on Fedora);
nothing else in wgaf does, which is why they are a separate workspace with their
own lockfile and a build step of their own.

| Application | What it is for |
|---|---|
| `window-test` | A fixed set of three windows: `wgaf window list`, `focus`, `resize`, `close` |
| `input-test` | A text entry and a button: `wgaf type`, `key press`/`release`, `mouse click`/`move`/`scroll` |
| `accessibility-test` | A tree of elements with fixed accessible names: `wgaf a11y find`, `tree`, `info`, `click`, `focus`, `set-text` |

## Running the tests that use them

```sh
make test-desktop
```

One command: it builds the applications and runs every suite that drives them,
through the shared harness in `wgaf-daemon/tests/harness/`.

They run against your live session, which is the point, and it takes a few
seconds. Two things follow from that. They are `#[ignore]`d, so a plain
`cargo test` never starts them; and they run single-threaded, because two suites
sharing one keyboard focus would each read the other's keystrokes.

Anything they need of the machine is checked up front — a Wayland session, a
writable `/dev/uinput`, an accessibility bus, the GNOME Shell Extension, a built
application — and reported as the missing requirement rather than as whatever
symptom shows up first.

`wgaf-daemon/tests/accessibility.rs` is the one suite here that synthesizes no
input at all: AT-SPI reaches an application directly, so it needs no keyboard
focus and cannot type into anything of yours. It still opens windows, so it
belongs with the rest.

## Why these exist

Tests used to drive `gtk4-demo`, an application this project does not control.
Their expectations were facts discovered by hand — that a particular entry
advertises no actions, that a particular view is read-only — which a GTK upgrade
can invalidate without anyone noticing, and they could not run at all where
`gtk4-demo` was not installed.

These applications draw a UI that is fixed and known, so a test failure means
wgaf changed rather than the toolkit did.

## The two rules

**wgaf drives, the application reports.** An assertion never travels back
through the code path being tested. `wgaf type "hello"` is verified by reading
the application's report file, never by asking `wgaf a11y info` what the
application contains. Otherwise a single bug in the accessibility layer can
produce a false pass, and a real failure blames the wrong subsystem.

**The chain stops here.** A test application draws a fixed UI and reports its
own state. It has no automation of its own — no compositor integration, no
D-Bus driving, no AT-SPI. That is what stops the regress: there is no "test the
test application" layer, because there is nothing in one to discover. If one
ever needs automating to verify, it has grown too big and should be split.

## The report file

Every application takes `--report <PATH>` and writes a single JSON object there,
rewriting it in full on every change it observes. The directory must already
exist. The shared `report` crate implements this; read its documentation before
writing a new application, and use it rather than inventing a second shape.

```json
{
  "app": "window-test",
  "seq": 9,
  "pid": 43576,
  "window_count": 3,
  "windows": [
    { "role": "main", "title": "wgaf window-test", "width": 640, "height": 480,
      "focused": true, "maximized": false, "fullscreen": false, "visible": true }
  ]
}
```

`app` and `seq` are the envelope every report carries; everything else is the
application's own state.

`accessibility-test`'s state is what makes accessibility testable under the
first rule at all. Every mutating `wgaf a11y` command has a field here that only
moves if the command actually reached the widget — `activate_count` for `click`,
`entry_text` for `set-text`, `focused_widget` for `focus`. It also reports
`deep_nesting` and `wide_item_count`, which describe the shape of its own tree,
so a test asserting that wgaf's default walk stops short of the bottom can check
that premise instead of assuming it.

`input-test`'s state is shaped for diagnosis rather than convenience. It reports
the entry's text *and* the raw key events side by side, because an empty event
log means the input never arrived while a populated log with unexpected text
means it arrived and the keyboard layout translated it — different faults, and
only one of them is wgaf's. Its event logs keep the most recent 64 entries, with
separate uncapped totals, so that a long `wgaf type` cannot produce a report too
large to read.

**`window_count` counts the windows `window-test` tracks, not the ones still
open.** It is three from the first report to the last; a closed window stays in
the array with `visible: false`. It is not a readiness signal and not a liveness
signal — the first report already has all three, before the compositor has
mapped any of them.

**`seq` is how a test knows when something happened.** No file means the
application has not started. An unchanged `seq` means it started and nothing has
happened since. An increased `seq` means a new event arrived. Take a baseline
once the application is up, drive wgaf, then wait for `seq` to exceed the
baseline — never wait for a particular value, because mapping the windows
already produces a burst of reports before a test does anything.

**Reports are written whole or not at all.** Each is written to a temporary
sibling file and then `rename(2)`d into place, so a test polling the file sees
either the previous report or the new one. A partial read would surface as a
JSON parse error and look like a wgaf bug.

## What a test cannot get from these applications

Some things are not missing features and will not be added:

- **A window's position.** A Wayland client is never told where the compositor
  put it — there is no getter in GTK4 and no request in the protocol. So
  `wgaf window move` cannot be checked against the application's own account of
  itself. The only source for a position is the GNOME Shell Extension's reply,
  and asserting on that would be verifying wgaf with wgaf.
- **Which window is focused at startup.** It varies between runs; the compositor
  decides. Assert the change `wgaf window focus` causes, not the state before
  it.
- **A key's *name*, where the layout decides it.** The key beside the left shift
  is whatever the local layout prints on it. Its hardware keycode does not vary,
  so assert on that instead. Tests about *text* are the other way round —
  `wgaf-daemon/tests/keyboard_layout.rs` asserts on the entry's contents.
- **A pointer motion matching the distance requested.** libinput applies pointer
  acceleration to relative motion, so `wgaf mouse move 50 0` does not move the
  pointer 50 logical pixels. `input-test`'s coordinates prove motion arrived and
  in which direction; they do not measure the argument.
- **A click aimed at a particular widget.** wgaf has no absolute pointer
  positioning yet, so a test cannot put the pointer on the button. `input-test`
  captures clicks across the whole window for that reason.
- **A successful `wgaf a11y focus`.** GTK4's AT-SPI bridge answers
  `Component.GrabFocus` with `NotSupported` for every widget, on every version
  measured so far, so no GTK4 application can demonstrate a focus grab.
  `accessibility-test` reports `focused_widget` anyway, so the assertion is
  ready the day that changes.

Three things about the toolkit are worth knowing before writing an
accessibility test, all of them measured on GTK 4.22.4 after a first draft
assumed otherwise:

- **A `GtkLabel` can be clicked through AT-SPI.** It implements `Action`, so it
  cannot stand for "an element that refuses an action". `accessibility-test`
  uses a container for that.
- **A `GtkCheckButton` cannot.** It implements no `Action` at all, so a check
  box cannot be toggled through the accessibility bus.
- **A button's accessible name is its visible label**, whatever name is set on
  it explicitly — and its child label carries the same name, so a search by name
  alone matches both. Filter by role as well.

One thing that *is* comparable, contrary to what this file previously said:
**the size an application reports and the size `wgaf window list` reports match
exactly.** These are client-side-decorated GTK4 windows, so the frame rectangle
Mutter tracks is the client's own surface. Measured for all three `window-test`
windows on 2026-07-29. Do not assume it holds for a server-side-decorated
window, though no test application here produces one.
