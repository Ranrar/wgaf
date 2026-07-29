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
- **An exact match between reported size and `wgaf window list`'s size.** The
  application reports its own logical size, while Mutter reports a frame
  rectangle. They are measurements of different things and need not be equal.
