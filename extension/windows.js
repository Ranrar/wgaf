/* windows.js
 *
 * Mutter/Meta window & workspace logic for the wgaf D-Bus bridge.
 *
 * This module is intentionally D-Bus-agnostic: it deals only in Meta.Window /
 * Meta.Workspace objects and plain JS record objects (numbers/strings/bools).
 * GVariant marshaling and D-Bus error-name mapping live in dbusInterface.js.
 * Keeping the split this way means this file could be unit-tested or reused
 * without dragging in Gio.DBusExportedObject at all.
 *
 * Verified against: GNOME Shell 50.1 / Mutter 18 (introspected directly via
 * `GI_TYPELIB_PATH=/usr/lib/x86_64-linux-gnu/mutter-18:/usr/lib/gnome-shell gjs`
 * against Meta-18.typelib / Shell-18.typelib on the target dev machine).
 *
 * API notes for GNOME 50 / Mutter 18:
 *
 * - `global.display.list_all_windows()` is used instead of the older
 *   `global.get_window_actors()` pattern mentioned in the roadmap.
 *   `get_window_actors()` returns Clutter actors (compositor-side painting
 *   state) - each actor has a `.meta_window` property, but it's the wrong
 *   abstraction for "every window with its logical geometry"; Meta.Window is
 *   the right level and `list_all_windows()` is the direct, idiomatic way to
 *   get all of them regardless of workspace. `get_window_actors()` still
 *   exists in Mutter 18 (not deprecated/removed) but is not the better choice
 *   here.
 * - `Meta.Window.get_id()` returns the window's X11 XID and is 0 for native
 *   Wayland clients - it must NOT be used as the D-Bus `id`.
 *   `get_stable_sequence()` is Mutter's documented protocol-agnostic,
 *   monotonically-assigned-at-creation identifier, and is what's exposed as
 *   `id` over D-Bus here. This is the single most important gotcha for
 *   anyone extending this file.
 * - Window focus is tracked via `global.display`'s `notify::focus-window`
 *   GObject property-change signal (fires once per focus change, globally)
 *   rather than connecting a `focus` signal on every individual Meta.Window -
 *   simpler bookkeeping, identical result. (Meta.Window does also expose a
 *   per-window `focus` signal, confirmed via introspection, but there is no
 *   reason to use it here.)
 * - Window close/unmanage is only observable per-window, via the
 *   `unmanaging` signal on each Meta.Window - there is no global
 *   "window-closed" signal on Meta.Display. Every window we've seen must be
 *   tracked and explicitly disconnected, both when it closes and in
 *   destroy() for any still-open windows when the extension is disabled.
 * - There is no standalone `Meta.Window.resize()` method in Mutter 18;
 *   resizing without moving means calling `move_resize_frame()` with the
 *   window's current frame position and the new size.
 * - `delete()` sends a graceful close request (WM_DELETE_WINDOW / xdg-shell
 *   close) - this is used for CloseWindow rather than `kill()`, which
 *   force-terminates the client process.
 * - Workspace mutation lives on `global.workspace_manager`
 *   (Meta.WorkspaceManager), not on the workspaces themselves:
 *   `append_new_workspace(activate, timestamp)`, `remove_workspace(ws,
 *   timestamp)` and `reorder_workspace(ws, newIndex)`. Only `activate` is a
 *   Meta.Workspace method. Verified by introspection against Meta-18.typelib.
 * - `remove_workspace()` silently declines to remove the last remaining
 *   workspace. That case is rejected by name here rather than left to fail
 *   invisibly.
 * - Whether GNOME manages the workspace count itself is the `dynamic-
 *   workspaces` GSetting, not anything on Meta.WorkspaceManager - hence the one
 *   Gio.Settings use in this otherwise pure-Meta file. It changes what
 *   AddWorkspace means and so is reported to callers rather than hidden.
 * - `maximize()` and `unmaximize()` take NO arguments in Mutter 18, and they
 *   always act on both axes. `set_maximize_flags()` looks like the way to ask
 *   for one axis and is not: it sets state without relaying out, and
 *   `maximize()` overwrites it. Measured - see setWindowMaximized(). Older
 *   Mutter took the flags as an argument to `maximize()` itself, so an example
 *   found elsewhere will not compile here, and a newer one that appears to do
 *   per-axis should be tested before it is believed.
 * - The window-state getters come in two shapes and both are used below. The
 *   plain state is exposed as GObject *properties* (`win.minimized`,
 *   `win.fullscreen`, `win.above`, `win.on_all_workspaces`,
 *   `win.maximized_horizontally`, `win.maximized_vertically`), which is what
 *   the record and every confirmation read. The *questions about* a window are
 *   methods (`can_minimize()`, `can_maximize()`,
 *   `is_always_on_all_workspaces()`), and those guard the operations.
 * - Raising and lowering cannot be confirmed with `get_layer()`. A raise moves
 *   a window within its layer and leaves the layer itself unchanged, so the
 *   getter reads identically before and after. `Meta.Display`'s
 *   `sort_windows_by_stacking(list)` - which returns the list bottom-to-top -
 *   is the only route to the actual order, and is what `restackWindow` checks.
 */

import Gio from 'gi://Gio';

import {confirmSettled} from './confirm.js';

/** Thrown when a D-Bus caller references a window `id` that doesn't exist
 * (already closed, or never existed). Translated to a named D-Bus error
 * (org.gnome.Shell.Extensions.Wgaf.Error.WindowNotFound) in dbusInterface.js.
 */
export class WindowNotFoundError extends Error {
    constructor(id) {
        super(`No window with id ${id}`);
        this.name = 'WindowNotFoundError';
        this.id = id;
    }
}

/** Thrown when a D-Bus caller references a workspace index that doesn't exist.
 * Translated to a named D-Bus error
 * (org.gnome.Shell.Extensions.Wgaf.Error.WorkspaceNotFound) in
 * dbusInterface.js.
 *
 * Deliberately separate from WindowNotFoundError: workspace indices shift when
 * a workspace is added, removed or reordered, so "index 4 does not exist" is a
 * routine thing for a script to hit and to want to handle differently from a
 * window having closed.
 */
export class WorkspaceNotFoundError extends Error {
    constructor(index, count) {
        super(`No workspace at index ${index} (there ${count === 1 ? 'is' : 'are'} ${count})`);
        this.name = 'WorkspaceNotFoundError';
        this.index = index;
        this.count = count;
    }
}

/** Thrown when a workspace operation was issued and the compositor did not
 * carry it out. Translated to a named D-Bus error
 * (org.gnome.Shell.Extensions.Wgaf.Error.OperationNotApplied) in
 * dbusInterface.js.
 *
 * Every mutating method below re-reads the state it changed before replying,
 * rather than returning as soon as the request has been sent - see the
 * "confirm, don't assume" note in the WorkspaceManager section.
 */
export class OperationNotAppliedError extends Error {
    constructor(what, expected, actual) {
        super(`${what}: expected ${expected}, got ${actual}`);
        this.name = 'OperationNotAppliedError';
    }
}

/** Thrown when a window will not do something it was asked to do, and says so
 * before anything is attempted - a dialog that declares it cannot be
 * maximized, or a window that is on every workspace for a reason unsticking
 * cannot undo. Translated to a named D-Bus error
 * (org.gnome.Shell.Extensions.Wgaf.Error.OperationNotSupported) in
 * dbusInterface.js.
 *
 * Deliberately NOT OperationNotAppliedError, which means the opposite thing:
 * that the request WAS issued and the compositor did not carry it out. Here
 * nothing is issued at all, because Mutter has already answered the question.
 * A caller can retry a not-applied operation and it may work; retrying this one
 * never will.
 */
export class OperationNotSupportedError extends Error {
    constructor(id, operation, reason) {
        super(`window ${id} cannot ${operation}: ${reason}`);
        this.name = 'OperationNotSupportedError';
        this.id = id;
    }
}

/** Which way a restack request moves a window.
 *
 * Kept as a pure function over strings so it can be unit-tested outside a
 * Shell.
 *
 * The daemon validates this string before it is ever sent, so an unrecognised
 * value here means a caller talking to the extension directly rather than
 * through wgaf. It is still rejected by name rather than defaulted to
 * something, because guessing which way someone meant to move a window is how
 * a window ends up somewhere nobody asked for.
 */
export function parseStacking(stacking) {
    if (stacking !== 'raise' && stacking !== 'lower')
        throw new Error(`unknown stacking direction '${stacking}' - expected 'raise' or 'lower'`);
    return stacking;
}

/** Turn Mutter's workspace-grid numbers into two usable ones.
 *
 * ---------------------------------------------------------------------------
 * WHY THIS IS NEEDED - MEASURED 2026-08-06, GNOME Shell 50.1 / Mutter 18
 * ---------------------------------------------------------------------------
 * `get_layout_columns()` returns **-1** on an ordinary GNOME session, and keeps
 * returning -1 as workspaces are added: measured at 1, 2, 3 and 4 workspaces,
 * with `get_layout_rows()` reporting 1 throughout. A column count of -1 is not
 * something a caller can compute with, and the whole point of reporting the
 * grid is working out what "the workspace to the right" means.
 *
 * **What is measured and what is inferred, stated separately, because Mutter
 * documents neither** (there is no gir installed and the typelib carries no doc
 * strings for these):
 *
 *  - MEASURED: the pair is (rows=1, columns=-1), constant across workspace
 *    counts, on a static-workspace session.
 *  - INFERRED: -1 is an "unbounded / as many as needed" sentinel rather than a
 *    real count. A negative number of columns has no other sensible reading,
 *    and GNOME 40+ does lay workspaces out in one horizontal row, which
 *    (rows=1, unbounded columns) describes exactly.
 *
 * So the sentinel is resolved from the workspace count here rather than passed
 * on. A caller gets two positive numbers that describe the same layout, and
 * nobody downstream has to know that -1 ever meant anything.
 */
export function resolveGrid(rows, columns, nWorkspaces) {
    // Never report a zero-sized grid for a session that has workspaces: a
    // consumer dividing by either number would fail on a value wgaf invented.
    const total = Math.max(nWorkspaces, 1);
    const known = value => Number.isInteger(value) && value > 0;

    if (known(rows) && known(columns))
        return {rows, columns};
    if (known(rows))
        return {rows, columns: Math.ceil(total / rows)};
    if (known(columns))
        return {rows: Math.ceil(total / columns), columns};

    // Neither is known. One row of everything - which is what GNOME actually
    // does, and what the measured (1, -1) pair resolves to anyway.
    return {rows: 1, columns: total};
}

/** Which of `stacked` contains the point `(x, y)`, topmost first.
 *
 * `stacked` is [{id, rect}, ...] in Mutter's stacking order, BOTTOM FIRST -
 * the order `Meta.Display.sort_windows_by_stacking()` returns, which is also
 * the order _stackingPosition() reads. So the answer is the LAST entry that
 * contains the point, not the first. Returns null when the point is over no
 * window at all, which is an ordinary answer: the desktop background, the top
 * bar, and a gap between windows are all real places for a pointer to be.
 *
 * ---------------------------------------------------------------------------
 * WHAT THIS IS AND IS NOT
 * ---------------------------------------------------------------------------
 * This is a RECTANGLE test against frame rects, not Clutter picking. It
 * answers "which window's frame would a click at this point belong to",
 * which is the question a targeting guard needs, and it is exact for the
 * ordinary rectangular case that guard exists to protect.
 *
 * Two places it is not the compositor's own answer, stated rather than
 * discovered later: a window with a non-rectangular or partly transparent
 * frame is treated as its full rectangle, and override-redirect surfaces
 * (menus, tooltips, drop-downs) are excluded by the caller - so a click that
 * would land on an open menu is attributed to the window behind it. Both
 * failure directions are toward "the pointer is over the target", so the
 * guard can be permissive in a corner case; it is never wrong in the
 * direction of refusing a click that would have worked.
 *
 * Boundaries are half-open on the right and bottom, matching how a rectangle
 * of width w starting at x covers columns x..x+w-1. Two windows edge to edge
 * therefore never both claim the shared boundary pixel.
 */
/** `Meta.WindowType` as strings, indexed by the enum's own numeric values.
 *
 * Written out rather than imported from Meta, for the reason `MaximizeFlags`
 * is: a top-level `import Meta` makes this file unloadable outside a Shell and
 * costs `extension/tests/` its only way to reach anything here. The order is
 * the enum's declaration order, verified against `Meta.WindowType` on
 * Mutter 18 — and `windowTypeName()` is pinned by a unit test against a
 * hardcoded copy of that list, so a reordering in a future Mutter shows up as
 * a failing test rather than as every window reporting the wrong kind.
 */
const WINDOW_TYPE_NAMES = [
    'normal', 'desktop', 'dock', 'dialog', 'modal_dialog', 'toolbar', 'menu',
    'utility', 'splashscreen', 'dropdown_menu', 'popup_menu', 'tooltip',
    'notification', 'combo', 'dnd', 'override_other',
];

/** The name for a `Meta.WindowType` value, or `'unknown'` for one this build
 * has never heard of.
 *
 * A new enum member in a future Mutter must not become an exception thrown
 * inside `ListWindows` — losing the whole window list because one window is of
 * an unfamiliar kind would be a far worse failure than not naming its type.
 */
export function windowTypeName(value) {
    return WINDOW_TYPE_NAMES[value] ?? 'unknown';
}

export function topmostAt(stacked, x, y) {
    for (let i = stacked.length - 1; i >= 0; i--) {
        const {id, rect} = stacked[i];
        if (x >= rect.x && x < rect.x + rect.width &&
            y >= rect.y && y < rect.y + rect.height)
            return id;
    }
    return null;
}

export class WindowManager {
    constructor() {
        this._display = global.display;
        this._workspaceManager = global.workspace_manager;

        // [ [object, handlerId], ... ] for signals connected directly on
        // global.display (or other long-lived singletons).
        this._globalSignalIds = [];

        // Meta.Window -> [handlerId, ...] for signals connected on
        // individual windows (tracked per-window since they come and go).
        this._windowSignalIds = new Map();

        this._emitCallback = null;
    }

    /**
     * Start emitting D-Bus-facing events. `emitCallback(signalName, payload)`
     * is invoked with plain JS payloads - a window record object for
     * WindowCreated, or a numeric stable-sequence id for WindowClosed /
     * WindowFocusChanged. GVariant conversion is the caller's job
     * (extension.js, via helpers exported from dbusInterface.js).
     */
    connectSignals(emitCallback) {
        this._emitCallback = emitCallback;

        this._trackGlobalSignal(this._display, 'window-created', (_display, metaWindow) => {
            this._trackWindow(metaWindow);
            this._emit('WindowCreated', this._windowToRecord(metaWindow));
        });

        this._trackGlobalSignal(this._display, 'notify::focus-window', () => {
            const win = this._display.focus_window;
            if (win)
                this._emit('WindowFocusChanged', win.get_stable_sequence());
        });

        // Windows that already exist at enable()-time also need `unmanaging`
        // tracked, or we'd never see WindowClosed for anything open before
        // the extension was enabled.
        for (const win of this._display.list_all_windows())
            this._trackWindow(win);
    }

    /** Disconnect every signal handler this instance holds. Must be called
     * from extension.js's disable() - this is the other half of
     * connectSignals() and is what prevents leaked handlers across
     * enable/disable cycles.
     */
    destroy() {
        for (const [obj, id] of this._globalSignalIds)
            obj.disconnect(id);
        this._globalSignalIds = [];

        for (const [win, ids] of this._windowSignalIds) {
            for (const id of ids) {
                try {
                    win.disconnect(id);
                } catch (e) {
                    // Window may already be fully unmanaged by Mutter;
                    // disconnecting a dead handler id is harmless to skip.
                }
            }
        }
        this._windowSignalIds.clear();

        this._emitCallback = null;
    }

    /** ListWindows: every window, across every workspace, as plain records. */
    listWindows() {
        return this._display.list_all_windows()
            // Override-redirect windows (tooltips, popup menus, combo
            // dropdowns, etc.) aren't things a user/automation script would
            // sensibly focus/move/resize/close - filtered out the same way
            // GNOME's own window-list style extensions do.
            .filter(win => !win.is_override_redirect())
            .map(win => this._windowToRecord(win));
    }

    /** GetWorkspaces: index/active/window-count for every workspace. */
    getWorkspaces() {
        const activeIndex = this._workspaceManager.get_active_workspace_index();
        const count = this._workspaceManager.get_n_workspaces();
        const workspaces = [];
        for (let i = 0; i < count; i++) {
            const ws = this._workspaceManager.get_workspace_by_index(i);
            workspaces.push({
                index: i,
                active: i === activeIndex,
                n_windows: ws.list_windows().length,
            });
        }
        return workspaces;
    }

    // --- Workspace layout and mutation --------------------------------------
    //
    // CONFIRM, DON'T ASSUME
    //
    // Every mutating method below re-reads the state it changed and only then
    // replies, via confirmSettled() (confirm.js). None of them assume the
    // change is visible the moment the Mutter call returns.
    //
    // This is not defensive padding. `warp_pointer()` was measured on this
    // compositor to take effect several milliseconds after its call returns
    // (see pointer.js), and a bridge method that replies before its effect is
    // readable hands the caller a race: `wgaf workspace switch 2` followed by
    // `wgaf window list` would report the windows of workspace 1. Whether any
    // particular one of these settles synchronously has NOT been measured -
    // the extension cannot be reloaded on Wayland without ending the session -
    // so the code is written not to care either way. A synchronous change
    // confirms on the first read and costs nothing.
    //
    // A change that never becomes readable raises OperationNotAppliedError
    // rather than replying successfully. "The switch did not happen" is
    // something a script must be able to see.

    /** GetWorkspaceLayout: how the workspaces are arranged, and whether GNOME
     * is managing their number itself.
     *
     * `dynamic` is the one that changes what the mutating methods below mean,
     * and it is why this is reported rather than left for a user to look up.
     * With dynamic workspaces on - the GNOME default - the Shell maintains
     * exactly one empty workspace at the end and reclaims any other that
     * empties. AddWorkspace still adds one; it may simply not survive being
     * left empty. Refusing the call would be wrong (the operation does work),
     * and staying silent would be worse (the workspace vanishing looks like a
     * wgaf bug), so the state is reported and the contract documented.
     *
     * Rows/columns describe the grid GNOME arranges workspaces in, which is
     * what "the workspace to the right" means. A vertical GNOME layout reports
     * one column.
     */
    getWorkspaceLayout() {
        const n = this._workspaceManager.get_n_workspaces();
        const {rows, columns} = resolveGrid(
            this._workspaceManager.get_layout_rows(),
            this._workspaceManager.get_layout_columns(),
            n
        );
        return {
            n_workspaces: n,
            active: this._workspaceManager.get_active_workspace_index(),
            rows,
            columns,
            dynamic: this._dynamicWorkspaces(),
        };
    }

    /** SwitchWorkspace: make the workspace at `index` the active one.
     *
     * Resolves once `get_active_workspace_index()` reports it, so a caller may
     * treat the reply as meaning "that workspace is active now".
     */
    switchWorkspace(index) {
        const ws = this._requireWorkspace(index);
        ws.activate(this._timestamp());

        return confirmSettled(
            () => this._workspaceManager.get_active_workspace_index(),
            active => active === index
        ).then(({value, confirmed}) => {
            if (!confirmed)
                throw new OperationNotAppliedError('workspace did not become active', index, value);
        });
    }

    /** AddWorkspace: append a workspace, resolving with its index.
     *
     * Appended rather than inserted, because that is the only thing Mutter
     * offers - `append_new_workspace` is the sole creation route, and a
     * caller wanting one elsewhere appends then reorders.
     *
     * `false` for `activate`: adding a workspace and jumping to it are two
     * decisions, and a caller who wanted both can say so with a second call.
     * Silently moving the user's view is the more surprising default.
     *
     * See getWorkspaceLayout() on what `dynamic` means for the result's
     * lifetime.
     */
    addWorkspace() {
        const before = this._workspaceManager.get_n_workspaces();
        this._workspaceManager.append_new_workspace(false, this._timestamp());

        return confirmSettled(
            () => this._workspaceManager.get_n_workspaces(),
            count => count > before
        ).then(({value, confirmed}) => {
            if (!confirmed)
                throw new OperationNotAppliedError('workspace was not added', `more than ${before}`, value);
            // The appended one is the last, and its index is the count minus
            // one - read back rather than assumed to be `before`, since under
            // dynamic workspaces the Shell may have adjusted the count too.
            return value - 1;
        });
    }

    /** RemoveWorkspace: remove the workspace at `index`.
     *
     * Mutter refuses to remove the last remaining workspace, and does so
     * silently, so that case is rejected here by name instead - a caller told
     * "no workspace was removed" would have no idea why.
     *
     * Windows on the removed workspace are not closed; Mutter moves them to a
     * neighbouring workspace, which is the same thing that happens when a user
     * removes one from the overview.
     */
    removeWorkspace(index) {
        const ws = this._requireWorkspace(index);
        const before = this._workspaceManager.get_n_workspaces();
        if (before <= 1) {
            throw new OperationNotAppliedError(
                'the last workspace cannot be removed', 'at least 2 workspaces', before);
        }
        this._workspaceManager.remove_workspace(ws, this._timestamp());

        return confirmSettled(
            () => this._workspaceManager.get_n_workspaces(),
            count => count < before
        ).then(({value, confirmed}) => {
            if (!confirmed)
                throw new OperationNotAppliedError('workspace was not removed', `fewer than ${before}`, value);
        });
    }

    /** ReorderWorkspace: move the workspace at `index` to `newIndex`.
     *
     * Every other workspace shifts to make room, so the indices a caller read
     * before this call are stale afterwards. That is Mutter's model, not a
     * choice made here.
     */
    reorderWorkspace(index, newIndex) {
        const ws = this._requireWorkspace(index);
        // Validated against the same bounds: reordering to a position that
        // does not exist is the same mistake as naming a workspace that does
        // not, and Mutter would otherwise clamp or ignore it silently.
        this._requireWorkspace(newIndex);
        this._workspaceManager.reorder_workspace(ws, newIndex);

        return confirmSettled(
            () => ws.index(),
            at => at === newIndex
        ).then(({value, confirmed}) => {
            if (!confirmed)
                throw new OperationNotAppliedError('workspace was not reordered', newIndex, value);
        });
    }

    /** GetWorkAreas: the usable area of each monitor - the screen minus the
     * top bar, docks, and anything else reserving space.
     *
     * WHY EACH ENTRY CARRIES THE MONITOR'S OWN GEOMETRY TOO
     *
     * `get_work_area_for_monitor()` takes a *Mutter* monitor index. The daemon
     * knows monitors by connector name, from
     * org.gnome.Mutter.DisplayConfig, and whether Mutter's indices enumerate
     * those in the same order is an assumption nobody here has measured. So
     * the index is not exposed at all: each entry reports the monitor's
     * rectangle, and the daemon matches on that. Two monitors cannot occupy
     * the same rectangle, so the match is exact where an index would have been
     * a guess.
     *
     * WHICH WORKSPACE
     *
     * The active one. Work areas are per-workspace in Mutter's API because
     * struts can in principle differ, though on GNOME the top bar is global
     * and they do not. Reporting the active workspace's is both the useful
     * answer and the one a caller can predict.
     */
    getWorkAreas() {
        const workspace = this._workspaceManager.get_active_workspace();
        const areas = [];
        for (let i = 0; i < this._display.get_n_monitors(); i++) {
            const monitor = this._display.get_monitor_geometry(i);
            const work = workspace.get_work_area_for_monitor(i);
            areas.push({
                x: monitor.x,
                y: monitor.y,
                width: monitor.width,
                height: monitor.height,
                work_area_x: work.x,
                work_area_y: work.y,
                work_area_width: work.width,
                work_area_height: work.height,
            });
        }
        return areas;
    }

    /** MoveWindowToWorkspace: send a window to another workspace.
     *
     * Resolves once the window reports itself on the target workspace, so a
     * caller may treat the reply as meaning it is there now.
     *
     * The window is moved, not followed: the active workspace is left alone, so
     * this sends a window away rather than taking the user with it. A caller
     * wanting both says so with a SwitchWorkspace of its own - the same split
     * AddWorkspace makes, and for the same reason.
     *
     * `change_workspace_by_index(index, append)` takes a second argument that
     * would create workspaces up to `index` if it does not exist. Passed as
     * false: the index is validated here first, so a caller naming a workspace
     * that is not there gets WorkspaceNotFound rather than silently causing new
     * workspaces to appear. Creating one is AddWorkspace's job, and it is gated
     * by its own capability.
     */
    moveWindowToWorkspace(id, index) {
        const win = this._requireWindow(id);
        this._requireWorkspace(index);
        win.change_workspace_by_index(index, false);

        return confirmSettled(
            () => {
                const ws = win.get_workspace();
                return ws ? ws.index() : -1;
            },
            at => at === index
        ).then(({value, confirmed}) => {
            if (!confirmed)
                throw new OperationNotAppliedError('window did not move workspace', index, value);
        });
    }

    /** Whether GNOME is managing the number of workspaces itself.
     *
     * Read from the same GSetting the Shell reads. Wrapped because a missing
     * or unreadable schema must not take out an otherwise working call: the
     * flag is advisory - it tells a caller how to interpret AddWorkspace's
     * result - and no operation here depends on it.
     */
    _dynamicWorkspaces() {
        try {
            return new Gio.Settings({schema_id: 'org.gnome.mutter'}).get_boolean('dynamic-workspaces');
        } catch (e) {
            logError(e, 'wgaf: could not read org.gnome.mutter dynamic-workspaces');
            return false;
        }
    }

    _requireWorkspace(index) {
        const count = this._workspaceManager.get_n_workspaces();
        if (!Number.isInteger(index) || index < 0 || index >= count)
            throw new WorkspaceNotFoundError(index, count);
        return this._workspaceManager.get_workspace_by_index(index);
    }

    focusWindow(id) {
        const win = this._requireWindow(id);
        win.activate(this._timestamp());
    }

    moveWindow(id, x, y) {
        const win = this._requireWindow(id);
        // `true` (user_op) marks this as a user-directed move so Mutter
        // applies its normal on-screen/edge constraints, matching how a real
        // drag behaves - appropriate since this is automation acting on the
        // user's behalf, not an internal/session-restore move.
        win.move_frame(true, x, y);
    }

    /** ResizeWindow: resize a window to `width` x `height`, keeping its
     * top-left corner where it is.
     *
     * The reply means the new size is READABLE, not merely requested.
     * `move_resize_frame()` returns as soon as Mutter has accepted the request,
     * and for roughly 30 ms afterwards `get_frame_rect()` - and therefore
     * `wgaf window list` - still reports the old rectangle. A script that
     * resizes and then reads back to compute a centre point aims at the old
     * one, which is how this became a filed defect rather than a curiosity.
     *
     * A REQUEST MUTTER CLAMPS IS REPORTED, NOT HIDDEN. The settle condition is
     * the requested size specifically, so a window with a minimum size larger
     * than the request - or one that refuses the resize outright - never
     * satisfies it and raises OperationNotAppliedError naming the size it
     * actually has. That is an ADR-0007 "unverified" outcome rather than a
     * failure: nothing malfunctioned, the window is simply not the size that
     * was asked for, and a caller that assumed otherwise needs to know. The
     * alternative - waiting for the geometry to stop moving and replying
     * successfully at whatever size it settled on - would report a clamped
     * resize as a successful one, which is the same untruth in a new place.
     */
    resizeWindow(id, width, height) {
        const win = this._requireWindow(id);
        const before = win.get_frame_rect();

        // Read ahead of the mutation, as every _confirmGeometrySettled() caller
        // must: a resize to the size a window already has changes nothing
        // observable, and waiting for a change that is not coming would report
        // it as unapplied.
        const alreadyThere = before.width === width && before.height === height;

        win.move_resize_frame(true, before.x, before.y, width, height);

        return this._confirmGeometrySettled(
            win,
            () => win.get_frame_rect(),
            rect => rect.width === width && rect.height === height,
            alreadyThere,
            before,
            'window did not resize',
            {
                expected: `${width}x${height}`,
                // Empty: _confirmGeometrySettled() already prints the frame
                // rect, and here that is the whole story.
                describe: () => '',
            }
        );
    }

    closeWindow(id) {
        const win = this._requireWindow(id);
        win.delete(this._timestamp());
    }

    // --- Window state -------------------------------------------------------
    //
    // Six operations that change what a window IS rather than where it is, and
    // they share three rules.
    //
    // ASK FIRST WHERE MUTTER WILL ANSWER. `can_minimize()`, `can_maximize()`
    // and `is_always_on_all_workspaces()` are cheap questions with real
    // answers, and a window that says no would otherwise absorb the request
    // and produce nothing. That is indistinguishable from wgaf being broken,
    // so it is refused by name up front - see OperationNotSupportedError.
    //
    // CONFIRM, DON'T ASSUME. Every one of them re-reads the state it changed
    // through confirmSettled() before replying, exactly as the workspace
    // mutations above do. Same reasoning, and the same
    // OperationNotAppliedError when the change never becomes readable.
    //
    // NO IMPLICIT SECOND OPERATION. Nothing here does a neighbouring
    // operation's job on the caller's behalf: unminimizing does not focus,
    // maximizing does not raise, and un-fullscreening does not restore a
    // remembered size. Each of those is its own call with its own capability,
    // the same split moveWindowToWorkspace() makes by not switching workspace.

    /** SetWindowMinimized: minimize or restore a window.
     *
     * Restoring does NOT focus the window. `wgaf window focus` is a separate
     * capability, and a script that wants both says so - see the section note
     * above.
     */
    setWindowMinimized(id, minimized) {
        const win = this._requireWindow(id);
        if (minimized && !win.can_minimize()) {
            throw new OperationNotSupportedError(
                id, 'be minimized', 'the window declares itself unminimizable');
        }

        if (minimized)
            win.minimize();
        else
            win.unminimize();

        return this._confirmFlag(
            () => win.minimized, minimized,
            minimized ? 'window did not minimize' : 'window did not unminimize');
    }

    /** SetWindowMaximized: maximize or unmaximize a window.
     *
     * ---------------------------------------------------------------------------
     * BOTH AXES, ALWAYS - AND THAT IS MUTTER'S LIMIT, NOT A SHORTCUT
     * ---------------------------------------------------------------------------
     * There is deliberately no per-axis argument, because Mutter 18 offers no
     * way to honour one. Measured 2026-08-07, inside the Shell, against a real
     * window:
     *
     *   baseline                              flags=0 h=false v=false  640x480
     *   after set_maximize_flags(HORIZONTAL)  flags=1 h=true  v=false  640x480
     *   after maximize()                      flags=3 h=true  v=true   2560x1408
     *
     * `set_maximize_flags()` is a **state setter, not a request**: it moves the
     * flags and the per-axis fields and never triggers a relayout - the window
     * is still 640x480 after it. `maximize()` then overwrites the flags to BOTH
     * and lays out full-screen, whatever they had been set to. The two per-axis
     * GObject properties are read-only (`Property
     * MetaWindowWayland.maximized-horizontally is not writable`), so they are
     * not a way round it either.
     *
     * The remaining option would be to set the flags and then place the window
     * with move_resize_frame() by hand, which means reimplementing maximization
     * and leaving Mutter believing it laid out a window it did not. Rejected -
     * see the note in backlog.md if a real route ever appears.
     */
    setWindowMaximized(id, maximized) {
        const win = this._requireWindow(id);

        // Only asked on the way in. `can_maximize()` answers whether a window
        // may be maximized at all; there is no equivalent question about
        // unmaximizing, and a window that is already unmaximized confirms
        // immediately anyway.
        if (maximized && !win.can_maximize()) {
            throw new OperationNotSupportedError(
                id, 'be maximized', 'the window declares itself unmaximizable');
        }

        // Read before the mutation: maximize() sets the state flags
        // synchronously, so afterwards there is nothing left to compare
        // against. See _confirmGeometrySettled().
        const alreadyThere = win.maximized_horizontally === maximized &&
            win.maximized_vertically === maximized;
        const before = win.get_frame_rect();

        if (maximized)
            win.maximize();
        else
            win.unmaximize();

        // Both axes are read, rather than one standing in for the pair: a
        // window left maximized on only one axis - which a user can produce
        // with GNOME's own keybindings even though wgaf cannot - is not
        // maximized, and reporting it as such would be the same untruth this
        // method was carrying before.
        //
        // Waits for the resize as well as the flags - see
        // _confirmGeometrySettled() for why the flags alone reply too early.
        return this._confirmGeometrySettled(
            win,
            () => ({
                horizontal: win.maximized_horizontally,
                vertical: win.maximized_vertically,
            }),
            state => state.horizontal === maximized && state.vertical === maximized,
            alreadyThere,
            before,
            maximized ? 'window did not maximize' : 'window did not unmaximize',
            {
                expected: maximized,
                describe: state =>
                    `horizontal = ${state.horizontal}, vertical = ${state.vertical}`,
            }
        );
    }

    /** SetWindowFullscreen: make a window fullscreen, or return it to its
     * previous size.
     *
     * Distinct from maximizing, and not a synonym for it: a fullscreen window
     * covers the top bar and any dock, where a maximized one stops at the work
     * area. Scripts positioning other windows afterwards care about the
     * difference.
     */
    setWindowFullscreen(id, fullscreen) {
        const win = this._requireWindow(id);

        // Captured ahead of the mutation, as in setWindowMaximized().
        const alreadyThere = win.fullscreen === fullscreen;
        const before = win.get_frame_rect();

        if (fullscreen)
            win.make_fullscreen();
        else
            win.unmake_fullscreen();

        // Geometry-settled rather than a bare flag read, for the same reason
        // maximizing is: `fullscreen` turns true when Mutter decides it, which
        // is before the window has been resized to cover the screen.
        return this._confirmGeometrySettled(
            win,
            () => win.fullscreen,
            state => state === fullscreen,
            alreadyThere,
            before,
            fullscreen ? 'window did not go fullscreen' : 'window did not leave fullscreen',
            {expected: fullscreen, describe: state => `fullscreen = ${state}`}
        );
    }

    /** SetWindowAbove: keep a window above other windows, or stop doing so.
     *
     * This moves the window between Mutter's stack layers, so it outranks
     * anything restackWindow() can do - a raised ordinary window still sits
     * below an always-on-top one.
     */
    setWindowAbove(id, above) {
        const win = this._requireWindow(id);

        if (above)
            win.make_above();
        else
            win.unmake_above();

        return this._confirmFlag(
            () => win.above, above,
            above ? 'window did not stay above' : 'window did not stop staying above');
    }

    /** SetWindowOnAllWorkspaces: show a window on every workspace, or return
     * it to just one.
     *
     * UNSTICKING LEAVES THE WINDOW ON THE ACTIVE WORKSPACE
     *
     * Not on whichever one it was on before it was stuck - that is not
     * remembered by anything. A window on every workspace is on the active one
     * too, so when it stops being on all of them, the one it keeps is the one
     * you are looking at. Measured 2026-08-07: stuck from workspace 0, viewed
     * from workspace 1, unstuck there, and it stayed on workspace 1.
     *
     * Worth stating because it makes unsticking a *move* for any caller that
     * switched workspace in between, and nothing about the call says so.
     *
     * Two getters answer nearby questions and they are not the same question.
     * `on_all_workspaces` is whether the window IS on all of them;
     * `is_always_on_all_workspaces()` is whether it is so for a reason that has
     * nothing to do with being stuck - Mutter puts windows there itself under
     * some multi-monitor configurations. Unsticking such a window changes
     * nothing, which would come back as a confirmation timeout and read like a
     * fault, so it is refused with the actual reason instead.
     */
    setWindowOnAllWorkspaces(id, onAllWorkspaces) {
        const win = this._requireWindow(id);
        if (!onAllWorkspaces && win.is_always_on_all_workspaces()) {
            throw new OperationNotSupportedError(
                id, 'be moved off all workspaces',
                'the compositor puts it on every workspace regardless of whether it is stuck');
        }

        if (onAllWorkspaces)
            win.stick();
        else
            win.unstick();

        return this._confirmFlag(
            () => win.on_all_workspaces, onAllWorkspaces,
            onAllWorkspaces
                ? 'window did not move to all workspaces'
                : 'window did not move off all workspaces');
    }

    /** RestackWindow: raise a window to the top of its stack layer, or lower
     * it to the bottom.
     *
     * WITHIN ITS LAYER, WHICH IS THE WHOLE ANSWER
     *
     * Mutter stacks windows in layers - desktop, bottom, normal, top, dock -
     * and raising moves a window to the top of ITS OWN layer, never past one.
     * So a raised ordinary window is still below an always-on-top window, and
     * that is Mutter's model rather than a limitation here. `setWindowAbove()`
     * is what changes layer.
     *
     * A raise does NOT focus, and focusing (`activate()`) does raise. They are
     * separate on purpose: raising a window to read it while typing somewhere
     * else is a thing a script may legitimately want.
     */
    restackWindow(id, stacking) {
        const win = this._requireWindow(id);
        const direction = parseStacking(stacking);

        if (direction === 'raise')
            win.raise();
        else
            win.lower();

        return this._confirmState(
            () => this._stackingPosition(win),
            position => (direction === 'raise' ? position.above : position.below) === 0,
            direction === 'raise' ? 'window was not raised' : 'window was not lowered',
            {
                expected: direction === 'raise' ? 'nothing above it' : 'nothing below it',
                describe: position => `${position.below} below, ${position.above} above`,
            }
        );
    }

    /** GetWindowAtPointer: which window the pointer is over right now.
     *
     * Returns `{found, id}` - `found: false` when the pointer is over no
     * window, with `id` meaningless in that case. A boolean rather than a
     * sentinel id, because every id this file hands out is a real
     * `get_stable_sequence()` and inventing a reserved one would put a
     * "0 means nothing" rule into every consumer.
     *
     * ---------------------------------------------------------------------------
     * THE POINTER IS READ HERE, NOT PASSED IN
     * ---------------------------------------------------------------------------
     * Deliberately, and it is the whole value of the method. A caller that read
     * the pointer separately and then asked what was under that coordinate
     * would have a gap between the two reads - and the pointer is the *user's*
     * pointer, which they can move at any moment. Reading position and
     * occupancy in one call inside the compositor makes the answer describe one
     * instant. The daemon's mouse-targeting guard depends on that; a stale
     * answer there means a click going somewhere nobody chose.
     *
     * ---------------------------------------------------------------------------
     * WHICH WINDOWS COUNT
     * ---------------------------------------------------------------------------
     * The same filters _stackingPosition() applies, for the same reasons, plus
     * visibility - a window the user cannot see cannot be under the pointer:
     *
     *  - **Not override-redirect.** Menus and tooltips, dropped as listWindows()
     *    drops them. See topmostAt()'s note on what that costs.
     *  - **Showing on its workspace**, and **on the workspace being viewed**.
     *    A minimized window still has a frame rect and would otherwise claim
     *    points it is nowhere near.
     */
    getWindowAtPointer() {
        const [x, y] = global.get_pointer();
        const activeIndex = this._workspaceManager.get_active_workspace_index();

        const visible = this._display.list_all_windows().filter(win => {
            if (win.is_override_redirect() || win.minimized || !win.showing_on_its_workspace())
                return false;
            if (win.on_all_workspaces)
                return true;
            const ws = win.get_workspace();
            return !!ws && ws.index() === activeIndex;
        });

        const stacked = this._display.sort_windows_by_stacking(visible).map(win => ({
            id: win.get_stable_sequence(),
            rect: win.get_frame_rect(),
        }));

        const id = topmostAt(stacked, x, y);
        return {found: id !== null, id: id === null ? 0 : id};
    }

    /** How many comparable windows sit below and above `win` in the stacking
     * order.
     *
     * WHAT "COMPARABLE" MEANS, AND WHY IT IS NARROWED
     *
     * Mutter's stacking order is global, and most of it is not something a
     * raise could ever change. Two filters make the count answer the question a
     * caller actually asked:
     *
     *  - **Same stack layer.** A raise cannot lift a window past a higher
     *    layer, so counting an always-on-top window as "above" would report a
     *    perfectly successful raise as having failed.
     *  - **Same workspace.** A window the user cannot see is neither above nor
     *    below anything from where they are sitting.
     *
     * Override-redirect windows are dropped for the reason listWindows() drops
     * them: tooltips and menus are not part of the order a script is arranging.
     *
     * Zero above therefore means "as raised as this window can get", which is
     * also true when it was already there - so a redundant raise confirms
     * immediately instead of timing out.
     */
    _stackingPosition(win) {
        const layer = win.get_layer();
        const peers = this._display.list_all_windows().filter(
            other =>
                !other.is_override_redirect() &&
                other.get_layer() === layer &&
                this._sharesWorkspace(other, win));

        const sorted = this._display.sort_windows_by_stacking(peers);
        // Matched on the stable sequence rather than object identity: GJS does
        // hand back the same wrapper for the same GObject, but the id is what
        // the rest of this file treats as a window's identity and it costs
        // nothing to stay consistent.
        const id = win.get_stable_sequence();
        const index = sorted.findIndex(other => other.get_stable_sequence() === id);
        if (index < 0) {
            // The window went away between the two reads. Reported as "not
            // settled" rather than thrown, so the caller gets
            // OperationNotApplied instead of an exception from inside a poll.
            return {below: -1, above: -1};
        }
        return {below: index, above: sorted.length - 1 - index};
    }

    /** Whether two windows are visible together - on the same workspace, or
     * one of them on all of them.
     */
    _sharesWorkspace(a, b) {
        if (a.on_all_workspaces || b.on_all_workspaces)
            return true;
        const wsA = a.get_workspace();
        const wsB = b.get_workspace();
        return !!wsA && !!wsB && wsA.index() === wsB.index();
    }

    /** Poll a boolean window property until it reaches `expected`.
     *
     * The tail of the four operations that flip one flag. The two that compare
     * something richer - a pair of axes, a position in the stacking order - go
     * straight to _confirmState() with their own predicate.
     */
    _confirmFlag(read, expected, what) {
        return this._confirmState(read, value => value === expected, what, {
            expected,
            describe: value => value,
        });
    }

    /** Poll `read()` until `isSettled` holds AND the window has stopped
     * resizing, and turn a change that never arrived into an
     * OperationNotAppliedError.
     *
     * ---------------------------------------------------------------------------
     * WHY THE STATE FLAG IS NOT ENOUGH
     * ---------------------------------------------------------------------------
     * Mutter sets `maximized_horizontally` / `fullscreen` when it *decides* the
     * window has that state, which is before the client has been reconfigured
     * and long before the new size is readable. The probe that settled the
     * per-axis question showed this directly:
     *
     *   after set_maximize_flags(HORIZONTAL)  flags=1 h=true v=false  rect=640x480
     *
     * True state, unchanged rectangle. So a confirmation that only reads the
     * flag replies while the window is still its old size, and the caller's
     * next `wgaf window list` gets the old rectangle - which is exactly the
     * `ResizeWindow` defect this whole design exists to avoid, arriving through
     * a different door. Caught by
     * `maximizing_changes_what_the_application_reports` on a real desktop, and
     * only when the suite ran as a whole: alone, it was slow enough to pass.
     *
     * ---------------------------------------------------------------------------
     * WHY "STABLE" ALONE IS ALSO NOT ENOUGH - THE FIRST FIX WAS WRONG
     * ---------------------------------------------------------------------------
     * The first attempt waited for two consecutive reads with the same frame
     * rect, and shipped, and failed the same test the next run. Two identical
     * reads cannot tell "has not started resizing yet" from "has finished
     * resizing":
     *
     *   poll 1   flags false   640x480   (no previous yet)
     *   poll 2   flags TRUE    640x480   same as poll 1 -> "stable" -> confirmed
     *
     * `maximize()` sets the flags synchronously, so by poll 2 the state matches
     * and the rectangle has not moved - and the check passed with the old size.
     * It only ever succeeded when the relayout happened to land inside one 2 ms
     * tick, which is why it passed once and then failed.
     *
     * So the rectangle must be seen to MOVE, and then hold still. `before` is
     * captured by the caller, ahead of the mutation, because by the time this
     * function runs the state has already changed.
     *
     * `alreadyThere` - also captured ahead of the mutation - is what keeps a
     * redundant call from waiting for a change that is never coming. Maximizing
     * an already-maximized window is a no-op with nothing to observe, and
     * timing it out would report a successful call as unapplied.
     *
     * The remaining gap, stated rather than hidden: a window whose maximized
     * size happens to equal its restored size would time out. No real window
     * does that - the work area is not 640x480 - but if one ever did, this
     * would call a working operation unapplied.
     */
    _confirmGeometrySettled(win, read, isSettled, alreadyThere, before, what, wording) {
        // Nothing to observe, and nothing to wait for.
        if (alreadyThere)
            return Promise.resolve();

        const same = (a, b) =>
            a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;

        let previous = null;
        return this._confirmState(
            () => ({state: read(), rect: win.get_frame_rect()}),
            value => {
                const moved = !same(before, value.rect);
                const stable = previous !== null && same(previous, value.rect);
                previous = value.rect;
                return isSettled(value.state) && moved && stable;
            },
            what,
            {
                expected: wording.expected,
                describe: value => {
                    const state = wording.describe(value.state);
                    const geometry = `${value.rect.width}x${value.rect.height}`;
                    // resizeWindow()'s state IS the geometry, so it describes
                    // itself as nothing rather than printing the same
                    // rectangle twice in one error message.
                    return state === '' ? geometry : `${state}, ${geometry}`;
                },
            },
            // Generous next to confirm.js's default, and for a different
            // reason: this waits on the *client* acknowledging a configure and
            // committing a new buffer, not on compositor state alone. A
            // toolkit relayout is not in the same order of magnitude as
            // reading a property.
            {timeoutMs: 500}
        );
    }

    /** Poll `read()` until `isSettled` holds, and turn a change that never
     * arrived into an OperationNotAppliedError.
     *
     * `expected` and `describe` exist only to word that error: the predicate
     * knows what it wants but cannot say it, and the value read may be a shape
     * rather than something worth printing raw.
     */
    _confirmState(read, isSettled, what, {expected, describe}, options = {}) {
        return confirmSettled(read, isSettled, options).then(({value, confirmed}) => {
            if (!confirmed)
                throw new OperationNotAppliedError(what, expected, describe(value));
        });
    }

    /** A timestamp Mutter will accept from a D-Bus call.
     *
     * `global.get_current_time()` is the usual idiom, and it is the wrong one
     * here. It returns the timestamp of the event currently being processed,
     * falling back to the display's last input-event time - and a D-Bus method
     * call is not an event, so on a session where no input has been handled
     * recently it yields 0.
     *
     * Mutter rejects 0 outright. `meta_display_ping_window()` starts with
     * `if (serial == 0) { g_warning("Tried to ping window %s with a bad
     * serial! Not allowed."); return; }`, and `meta_window_delete()` pings the
     * window to decide whether it is alive - so every CloseWindow issued this
     * way logged that warning in the compositor's journal and skipped the
     * liveness check. `activate()` takes the same timestamp for
     * focus-stealing prevention, where a 0 can mean the request is ignored.
     *
     * `get_current_time_roundtrip()` is defined for exactly this case. On
     * Wayland it returns `g_get_monotonic_time() / 1000`, which is always
     * non-zero and always fresh.
     */
    _timestamp() {
        return this._display.get_current_time_roundtrip();
    }

    _requireWindow(id) {
        const win = this._findWindow(id);
        if (!win)
            throw new WindowNotFoundError(id);
        return win;
    }

    _findWindow(id) {
        for (const win of this._display.list_all_windows()) {
            if (win.get_stable_sequence() === id)
                return win;
        }
        return null;
    }

    _windowToRecord(win) {
        const rect = win.get_frame_rect();
        const workspace = win.get_workspace();
        return {
            id: win.get_stable_sequence(),
            title: win.get_title() || '',
            app_id: win.get_wm_class() || '',
            workspace: workspace ? workspace.index() : -1,
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
            focused: win.has_focus(),
            maximized: win.is_maximized(),
            // The four states the window-state operations set. Reported here
            // so each one can be read back as well as written - without them a
            // script could minimize a window and have no way to ask whether it
            // is minimized.
            //
            // `on_all_workspaces` is the effective answer, so it is true both
            // for a window that was stuck and for one Mutter puts everywhere by
            // itself. Which of the two it is only matters when trying to
            // UNstick it, and setWindowOnAllWorkspaces() asks
            // is_always_on_all_workspaces() at that point rather than making
            // every caller carry a second field for the rare case.
            minimized: win.minimized,
            fullscreen: win.fullscreen,
            above: win.above,
            on_all_workspaces: win.on_all_workspaces,
            ...this._identity(win),
            ...this._geometryDetail(win),
        };
    }

    /** Who this window belongs to: the three answers that are not `app_id`.
     *
     * ---------------------------------------------------------------------------
     * `app_id` IS `get_wm_class()`, WHICH IS WHY THERE IS NO `wm_class` HERE
     * ---------------------------------------------------------------------------
     * See the field above. Reporting `wm_class` as well would print the same
     * string under two names, and the open issue about `window-test`'s dialog
     * disagreeing with its siblings is *about* that string — a second copy of it
     * disagrees identically.
     *
     * `gtk_application_id` is the one that can tell them apart: a window's class
     * is the window's, but the application id belongs to the GtkApplication, so
     * a dialog that never joined one should still report the application's id.
     * `wm_class_instance` comes along because it is the remaining distinct
     * identity Mutter holds and costs one call.
     *
     * All three are empty strings rather than null when absent. The wire format
     * is `a{sv}` and a missing key would make every consumer branch; an empty
     * string is the same "nothing here" and needs no branch.
     */
    _identity(win) {
        return {
            gtk_application_id: win.get_gtk_application_id() || '',
            wm_class_instance: win.get_wm_class_instance() || '',
            // Flatpak/Snap identity. An `app_id` alone misleads for a sandboxed
            // application, which is precisely the case where knowing what is
            // really running matters.
            sandboxed_app_id: win.get_sandboxed_app_id() || '',
            // 0 when Mutter does not know it, which it genuinely may not for a
            // remote or reparented client. Not -1: the field is unsigned on the
            // wire, and "no pid" and "pid 0" are the same statement.
            pid: Math.max(win.get_pid(), 0),
            window_type: windowTypeName(win.get_window_type()),
            // The window this one is a dialog *of*, or 0 for none. Ids are
            // stable sequences and Mutter starts them above zero, so 0 is free
            // to mean "none" without a second field to say so.
            transient_for: win.get_transient_for()?.get_stable_sequence() ?? 0,
        };
    }

    /** The geometry a caller cannot get from the frame rectangle alone.
     *
     * ---------------------------------------------------------------------------
     * THE MONITOR IS SENT AS A RECTANGLE, NOT AS MUTTER'S INDEX
     * ---------------------------------------------------------------------------
     * `get_monitor()` returns Mutter's own monitor index, and **there is no way
     * to look that up in `wgaf monitor list`** — that list comes from
     * `org.gnome.Mutter.DisplayConfig`, which enumerates connectors, and W18.2
     * established that the two orderings cannot be assumed to match (it could
     * not be verified from outside the compositor, and pairing them by position
     * would attach the wrong monitor to the wrong window while looking
     * plausible). An index the user cannot resolve is the `-1 columns` mistake
     * again: a number that reaches them as if it meant something.
     *
     * So the monitor's *rectangle* goes on the wire and the daemon matches it
     * against the layout it already reads, reporting a connector name. Exactly
     * what `GetWorkAreas` does, for exactly that reason. Two monitors cannot
     * occupy one rectangle, so the match is exact rather than a guess.
     *
     * `buffer_*` is the window including its shadow and any client-side
     * decoration, where the frame rect is what a user would point at. The
     * difference is the inset that coordinate arithmetic gets wrong.
     */
    _geometryDetail(win) {
        const buffer = win.get_buffer_rect();
        const monitor = this._display.get_monitor_geometry(win.get_monitor());

        return {
            buffer_x: buffer.x,
            buffer_y: buffer.y,
            buffer_width: buffer.width,
            buffer_height: buffer.height,
            monitor_x: monitor ? monitor.x : 0,
            monitor_y: monitor ? monitor.y : 0,
            monitor_width: monitor ? monitor.width : 0,
            monitor_height: monitor ? monitor.height : 0,
            // Snapped against another window, side by side. Mutter models it as
            // a link to the window sharing the split, so its presence is the
            // state and there is no separate flag to read.
            tiled: win.get_tile_match() !== null,
        };
    }

    _trackGlobalSignal(obj, signal, handler) {
        const id = obj.connect(signal, handler);
        this._globalSignalIds.push([obj, id]);
    }

    _trackWindow(win) {
        if (this._windowSignalIds.has(win))
            return;
        const id = win.connect('unmanaging', () => {
            this._untrackWindow(win);
            this._emit('WindowClosed', win.get_stable_sequence());
        });
        this._windowSignalIds.set(win, [id]);
    }

    _untrackWindow(win) {
        const ids = this._windowSignalIds.get(win);
        if (!ids)
            return;
        for (const id of ids) {
            try {
                win.disconnect(id);
            } catch (e) {
                // already disconnected/unmanaged
            }
        }
        this._windowSignalIds.delete(win);
    }

    _emit(signalName, payload) {
        if (this._emitCallback)
            this._emitCallback(signalName, payload);
    }
}
