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
 */

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

    focusWindow(id) {
        const win = this._requireWindow(id);
        win.activate(global.get_current_time());
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
        win.delete(global.get_current_time());
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
