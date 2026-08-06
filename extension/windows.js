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

    resizeWindow(id, width, height) {
        const win = this._requireWindow(id);
        const rect = win.get_frame_rect();
        win.move_resize_frame(true, rect.x, rect.y, width, height);
    }

    closeWindow(id) {
        const win = this._requireWindow(id);
        win.delete(this._timestamp());
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
