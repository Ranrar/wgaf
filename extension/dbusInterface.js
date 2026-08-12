/* dbusInterface.js
 *
 * D-Bus contract for the wgaf GNOME Shell Extension bridge, and the GVariant
 * marshaling/error-translation glue between it and windows.js's plain-JS
 * WindowManager. This file is the only place that knows about
 * Gio.DBusExportedObject, GLib.Variant, or D-Bus error names - windows.js
 * stays pure Mutter/Meta logic.
 *
 * ---------------------------------------------------------------------------
 * VERSIONING / NEGOTIATION STRATEGY
 * ---------------------------------------------------------------------------
 * The interface name is explicitly versioned: org.gnome.Shell.Extensions.Wgaf.V1
 *
 * This IS the version negotiation mechanism for this bridge:
 *
 *  - A future incompatible change (removing/renaming a method or argument,
 *    changing a field's type, changing signal argument shape) ships as a
 *    *new*, additional interface - org.gnome.Shell.Extensions.Wgaf.V2 -
 *    exported on the SAME object path alongside V1, rather than mutating V1's
 *    shape in place. Old daemon builds that only know about V1 keep working
 *    unchanged; new daemon builds can prefer V2 when it's present.
 *  - The Rust daemon should discover which interface(s) are available by
 *    calling org.freedesktop.DBus.Introspectable.Introspect on
 *    /org/gnome/Shell/Extensions/Wgaf and checking which
 *    org.gnome.Shell.Extensions.Wgaf.VN interface node(s) are listed - NOT by
 *    just trying a method call and hoping. "V1 interface not present in the
 *    introspection data" should be treated as "extension not installed, not
 *    enabled, or needs upgrading," surfaced as a clear, actionable daemon-side
 *    error - not a raw D-Bus timeout/"unknown method" exception.
 *  - Purely additive changes (new method, new signal, a new optional a{sv}
 *    field on an existing record) do NOT require a new interface/version -
 *    existing daemon clients simply ignore fields/methods/signals they don't
 *    know about, and a{sv} dicts are inherently forward-compatible for new
 *    fields. Only breaking changes require V2.
 * ---------------------------------------------------------------------------
 */

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {
    OperationNotAppliedError,
    OperationNotSupportedError,
    WindowNotFoundError,
    WorkspaceNotFoundError,
} from './windows.js';

export const DBUS_BUS_NAME = 'org.gnome.Shell.Extensions.Wgaf';
export const DBUS_OBJECT_PATH = '/org/gnome/Shell/Extensions/Wgaf';
export const DBUS_INTERFACE_NAME = 'org.gnome.Shell.Extensions.Wgaf.V1';

const ERROR_PREFIX = 'org.gnome.Shell.Extensions.Wgaf.Error';

/** D-Bus error names returned by this interface (org.freedesktop.DBus.Error
 * style, reverse-DNS strings) - the daemon should match on these rather than
 * parsing error message text.
 */
export const DBusErrors = {
    WINDOW_NOT_FOUND: `${ERROR_PREFIX}.WindowNotFound`,
    WORKSPACE_NOT_FOUND: `${ERROR_PREFIX}.WorkspaceNotFound`,
    OPERATION_NOT_APPLIED: `${ERROR_PREFIX}.OperationNotApplied`,
    OPERATION_NOT_SUPPORTED: `${ERROR_PREFIX}.OperationNotSupported`,
};

export const DBUS_INTERFACE_XML = `
<node>
  <interface name="${DBUS_INTERFACE_NAME}">
    <method name="ListWindows">
      <arg type="aa{sv}" direction="out" name="windows"/>
    </method>
    <method name="FocusWindow">
      <arg type="u" direction="in" name="id"/>
    </method>
    <method name="MoveWindow">
      <arg type="u" direction="in" name="id"/>
      <arg type="i" direction="in" name="x"/>
      <arg type="i" direction="in" name="y"/>
    </method>
    <method name="ResizeWindow">
      <arg type="u" direction="in" name="id"/>
      <arg type="i" direction="in" name="width"/>
      <arg type="i" direction="in" name="height"/>
    </method>
    <method name="CloseWindow">
      <arg type="u" direction="in" name="id"/>
    </method>
    <method name="GetWorkspaces">
      <arg type="aa{sv}" direction="out" name="workspaces"/>
    </method>
    <method name="GetWorkspaceLayout">
      <arg type="a{sv}" direction="out" name="layout"/>
    </method>
    <method name="SwitchWorkspace">
      <arg type="i" direction="in" name="index"/>
    </method>
    <method name="AddWorkspace">
      <arg type="i" direction="out" name="index"/>
    </method>
    <method name="RemoveWorkspace">
      <arg type="i" direction="in" name="index"/>
    </method>
    <method name="ReorderWorkspace">
      <arg type="i" direction="in" name="index"/>
      <arg type="i" direction="in" name="new_index"/>
    </method>
    <method name="MoveWindowToWorkspace">
      <arg type="u" direction="in" name="id"/>
      <arg type="i" direction="in" name="index"/>
    </method>
    <method name="SetWindowMinimized">
      <arg type="u" direction="in" name="id"/>
      <arg type="b" direction="in" name="minimized"/>
    </method>
    <method name="SetWindowMaximized">
      <arg type="u" direction="in" name="id"/>
      <arg type="b" direction="in" name="maximized"/>
    </method>
    <method name="SetWindowFullscreen">
      <arg type="u" direction="in" name="id"/>
      <arg type="b" direction="in" name="fullscreen"/>
    </method>
    <method name="SetWindowAbove">
      <arg type="u" direction="in" name="id"/>
      <arg type="b" direction="in" name="above"/>
    </method>
    <method name="SetWindowOnAllWorkspaces">
      <arg type="u" direction="in" name="id"/>
      <arg type="b" direction="in" name="on_all_workspaces"/>
    </method>
    <method name="RestackWindow">
      <arg type="u" direction="in" name="id"/>
      <arg type="s" direction="in" name="stacking"/>
    </method>
    <method name="GetWorkAreas">
      <arg type="aa{sv}" direction="out" name="work_areas"/>
    </method>
    <method name="WarpPointer">
      <arg type="i" direction="in" name="x"/>
      <arg type="i" direction="in" name="y"/>
      <arg type="i" direction="out" name="actual_x"/>
      <arg type="i" direction="out" name="actual_y"/>
    </method>
    <method name="GetPointer">
      <arg type="i" direction="out" name="x"/>
      <arg type="i" direction="out" name="y"/>
    </method>
    <method name="GetWindowAtPointer">
      <arg type="b" direction="out" name="found"/>
      <arg type="u" direction="out" name="id"/>
    </method>
    <signal name="WindowCreated">
      <arg type="a{sv}" name="window"/>
    </signal>
    <signal name="WindowClosed">
      <arg type="u" name="id"/>
    </signal>
    <signal name="WindowFocusChanged">
      <arg type="u" name="id"/>
    </signal>
  </interface>
</node>`;

/**
 * Window record field shape (also the shape of the WindowCreated signal's
 * payload): id (u), title (s), app_id (s), workspace (i), x/y/width/height
 * (i), focused (b), maximized (b), minimized (b), fullscreen (b), above (b),
 * on_all_workspaces (b). `id` is Meta.Window's stable sequence number, not its
 * (Wayland-unsafe) X11 XID - see windows.js.
 */
function windowRecordToVariantDict(record) {
    return {
        id: new GLib.Variant('u', record.id),
        title: new GLib.Variant('s', record.title),
        app_id: new GLib.Variant('s', record.app_id),
        workspace: new GLib.Variant('i', record.workspace),
        x: new GLib.Variant('i', record.x),
        y: new GLib.Variant('i', record.y),
        width: new GLib.Variant('i', record.width),
        height: new GLib.Variant('i', record.height),
        focused: new GLib.Variant('b', record.focused),
        maximized: new GLib.Variant('b', record.maximized),
        minimized: new GLib.Variant('b', record.minimized),
        fullscreen: new GLib.Variant('b', record.fullscreen),
        above: new GLib.Variant('b', record.above),
        on_all_workspaces: new GLib.Variant('b', record.on_all_workspaces),
        gtk_application_id: new GLib.Variant('s', record.gtk_application_id),
        wm_class_instance: new GLib.Variant('s', record.wm_class_instance),
        sandboxed_app_id: new GLib.Variant('s', record.sandboxed_app_id),
        pid: new GLib.Variant('u', record.pid),
        window_type: new GLib.Variant('s', record.window_type),
        transient_for: new GLib.Variant('u', record.transient_for),
        buffer_x: new GLib.Variant('i', record.buffer_x),
        buffer_y: new GLib.Variant('i', record.buffer_y),
        buffer_width: new GLib.Variant('i', record.buffer_width),
        buffer_height: new GLib.Variant('i', record.buffer_height),
        monitor_x: new GLib.Variant('i', record.monitor_x),
        monitor_y: new GLib.Variant('i', record.monitor_y),
        monitor_width: new GLib.Variant('i', record.monitor_width),
        monitor_height: new GLib.Variant('i', record.monitor_height),
        tiled: new GLib.Variant('b', record.tiled),
    };
}

/** Workspace record field shape: index (i), active (b), n_windows (i). */
function workspaceRecordToVariantDict(record) {
    return {
        index: new GLib.Variant('i', record.index),
        active: new GLib.Variant('b', record.active),
        n_windows: new GLib.Variant('i', record.n_windows),
    };
}

/** Workspace layout field shape: n_workspaces (i), active (i), rows (i),
 * columns (i), dynamic (b).
 *
 * Describes the set of workspaces rather than any one of them, which is why it
 * is a single dict and not another entry in the array above.
 */
function workspaceLayoutToVariantDict(layout) {
    return {
        n_workspaces: new GLib.Variant('i', layout.n_workspaces),
        active: new GLib.Variant('i', layout.active),
        rows: new GLib.Variant('i', layout.rows),
        columns: new GLib.Variant('i', layout.columns),
        dynamic: new GLib.Variant('b', layout.dynamic),
    };
}

// --- Signal payload builders, used by extension.js when emitting on the
// exported D-Bus object. emit_signal() needs a single GVariant matching the
// signal's full argument tuple, unlike method return values (which
// Gio.DBusExportedObject marshals automatically from the interface XML's
// declared out-arg types, aside from 'v' leaves which must be real
// GLib.Variant instances - see windowRecordToVariantDict above).

export function windowCreatedSignalVariant(record) {
    return new GLib.Variant('(a{sv})', [windowRecordToVariantDict(record)]);
}

export function windowClosedSignalVariant(id) {
    return new GLib.Variant('(u)', [id]);
}

export function windowFocusChangedSignalVariant(id) {
    return new GLib.Variant('(u)', [id]);
}

/** Work-area record field shape: the monitor's own rectangle (x, y, width,
 * height - all i) plus its usable sub-rectangle (work_area_x, work_area_y,
 * work_area_width, work_area_height - all i).
 *
 * There is no monitor index or connector name here on purpose - see
 * WindowManager.getWorkAreas() for why the geometry is the identity.
 */
function workAreaToVariantDict(record) {
    return {
        x: new GLib.Variant('i', record.x),
        y: new GLib.Variant('i', record.y),
        width: new GLib.Variant('i', record.width),
        height: new GLib.Variant('i', record.height),
        work_area_x: new GLib.Variant('i', record.work_area_x),
        work_area_y: new GLib.Variant('i', record.work_area_y),
        work_area_width: new GLib.Variant('i', record.work_area_width),
        work_area_height: new GLib.Variant('i', record.work_area_height),
    };
}

/** The D-Bus error name for an exception thrown out of WindowManager, or null
 * for anything this file does not recognize.
 *
 * The daemon matches on these names, never on message text - see
 * `translate_window_error` in wgaf-daemon/src/windows/mod.rs.
 */
function errorName(error) {
    if (error instanceof WindowNotFoundError)
        return DBusErrors.WINDOW_NOT_FOUND;
    if (error instanceof WorkspaceNotFoundError)
        return DBusErrors.WORKSPACE_NOT_FOUND;
    if (error instanceof OperationNotAppliedError)
        return DBusErrors.OPERATION_NOT_APPLIED;
    if (error instanceof OperationNotSupportedError)
        return DBusErrors.OPERATION_NOT_SUPPORTED;
    return null;
}

/** Fail a D-Bus invocation with a named error the daemon can match on.
 *
 * ---------------------------------------------------------------------------
 * WHY NOT return_gerror(), AND WHY NOT THROW
 * ---------------------------------------------------------------------------
 * Both lose the name. Measured on this machine (GLib 2.86 / gjs 1.86,
 * 2026-08-07) rather than reasoned about:
 *
 *   Gio.DBusError.new_for_dbus_error('org.gnome.Shell.Extensions.Wgaf.Error.WindowNotFound', ...)
 *     is_remote_error:  true
 *     get_remote_error: org.gnome.Shell.Extensions.Wgaf.Error.WindowNotFound
 *     encode_gerror:    org.gtk.GDBus.UnmappedGError.Quark._g_2dio_2derror_2dquark.Code36
 *
 * `return_gerror()` replies with `encode_gerror()`'s answer, which is the third
 * line - the name is thrown away and the caller receives a generic unmapped
 * GError with the real name buried in the message text. Throwing is no better:
 * gjs's `_handleDBusError` sends a GLib.Error through `return_gerror()` for
 * exactly the same result, and logs a warning into the compositor's journal on
 * the way past.
 *
 * `return_dbus_error(name, message)` is the one path that puts the name on the
 * wire intact, so it is the only one used here. Everything that can fail with a
 * named error therefore replies through this function rather than throwing -
 * which is why the mutating methods below are all `<Name>Async` handlers even
 * when the work they do is synchronous.
 *
 * Unrecognized exceptions become org.freedesktop.DBus.Error.Failed with the
 * message attached, so a bug in windows.js still produces a reply rather than
 * leaving the caller waiting.
 */
function failInvocation(invocation, error) {
    const name = errorName(error);
    if (name)
        invocation.return_dbus_error(name, error.message);
    else
        invocation.return_error_literal(Gio.DBusError, Gio.DBusError.FAILED, `wgaf: ${error.message}`);
}

/** Run `work()` and complete the D-Bus invocation from its result, whether it
 * finishes immediately or returns a Promise.
 *
 * `toVariant` builds the reply tuple for a method that returns something;
 * omitting it replies with the empty tuple, which is what a method with no
 * out-args needs.
 *
 * `work` is a function rather than an already-started Promise so that a
 * *synchronous* throw is caught here too. Several WindowManager methods
 * validate their arguments before doing anything asynchronous - a window id
 * that does not exist, a window that declares it cannot be maximized - and
 * those throw before any Promise is constructed.
 *
 * The asynchronous half matters for the same reason it always did: an
 * unhandled rejection inside a `<Name>Async` handler is invisible, because
 * gjs's exception handling covers a thrown exception and not a rejected
 * Promise, so the caller would wait forever for a reply. Both paths must
 * terminate the invocation, and both go through failInvocation() so the error
 * name survives.
 */
function replyWhenSettled(invocation, work, toVariant = null) {
    let result;
    try {
        result = work();
    } catch (error) {
        failInvocation(invocation, error);
        return;
    }

    Promise.resolve(result).then(value => {
        invocation.return_value(toVariant ? toVariant(value) : null);
    }).catch(error => failInvocation(invocation, error));
}

/**
 * The JS object wrapped by Gio.DBusExportedObject.wrapJSObject(). Each method
 * here corresponds 1:1 to a method in DBUS_INTERFACE_XML; all Mutter/Meta
 * work is delegated to the injected WindowManager (windows.js) - this class
 * only marshals inputs/outputs and translates errors.
 */
export class WgafDBusInterface {
    constructor(windowManager, pointerManager) {
        this._wm = windowManager;
        this._pointer = pointerManager;
    }

    ListWindows() {
        return this._wm.listWindows().map(windowRecordToVariantDict);
    }

    /* Everything that can fail with a named error is a `<Name>Async` handler,
     * including the four below whose work is synchronous.
     *
     * Not a style choice: `return_dbus_error()` is the only reply path that
     * keeps a D-Bus error name intact, and reaching it needs the `invocation`
     * object, which a plain synchronous method never sees. Throwing instead
     * routes the error through gjs's `return_gerror()` and the daemon receives
     * `org.gtk.GDBus.UnmappedGError...` - see failInvocation() for the
     * measurements.
     *
     * As everywhere else here, the `Async` suffix is Gio.DBusExportedObject's
     * dispatch convention and does NOT appear in the interface XML: these are
     * `FocusWindow`, `MoveWindow`, `ResizeWindow` and `CloseWindow` on the bus.
     */

    FocusWindowAsync(params, invocation) {
        const [id] = params;
        replyWhenSettled(invocation, () => this._wm.focusWindow(id));
    }

    MoveWindowAsync(params, invocation) {
        const [id, x, y] = params;
        replyWhenSettled(invocation, () => this._wm.moveWindow(id, x, y));
    }

    ResizeWindowAsync(params, invocation) {
        const [id, width, height] = params;
        replyWhenSettled(invocation, () => this._wm.resizeWindow(id, width, height));
    }

    CloseWindowAsync(params, invocation) {
        const [id] = params;
        replyWhenSettled(invocation, () => this._wm.closeWindow(id));
    }

    GetWorkspaces() {
        return this._wm.getWorkspaces().map(workspaceRecordToVariantDict);
    }

    GetWorkspaceLayout() {
        return workspaceLayoutToVariantDict(this._wm.getWorkspaceLayout());
    }

    GetWorkAreas() {
        return this._wm.getWorkAreas().map(workAreaToVariantDict);
    }

    /* The four workspace mutations below are asynchronous for a second reason
     * on top of the one above: each confirms its effect is readable before
     * replying (see confirm.js), so none of them could be a plain synchronous
     * method even if the error path allowed it.
     */

    SwitchWorkspaceAsync(params, invocation) {
        const [index] = params;
        replyWhenSettled(invocation, () => this._wm.switchWorkspace(index));
    }

    AddWorkspaceAsync(params, invocation) {
        replyWhenSettled(
            invocation,
            () => this._wm.addWorkspace(),
            index => new GLib.Variant('(i)', [index])
        );
    }

    RemoveWorkspaceAsync(params, invocation) {
        const [index] = params;
        replyWhenSettled(invocation, () => this._wm.removeWorkspace(index));
    }

    ReorderWorkspaceAsync(params, invocation) {
        const [index, newIndex] = params;
        replyWhenSettled(invocation, () => this._wm.reorderWorkspace(index, newIndex));
    }

    MoveWindowToWorkspaceAsync(params, invocation) {
        const [id, index] = params;
        replyWhenSettled(invocation, () => this._wm.moveWindowToWorkspace(id, index));
    }

    /* The six window-state operations. Each confirms the state it changed
     * before replying, and each can refuse up front when Mutter says the
     * window will not do it - see the "Window state" section of windows.js.
     */

    SetWindowMinimizedAsync(params, invocation) {
        const [id, minimized] = params;
        replyWhenSettled(invocation, () => this._wm.setWindowMinimized(id, minimized));
    }

    SetWindowMaximizedAsync(params, invocation) {
        const [id, maximized] = params;
        replyWhenSettled(invocation, () => this._wm.setWindowMaximized(id, maximized));
    }

    SetWindowFullscreenAsync(params, invocation) {
        const [id, fullscreen] = params;
        replyWhenSettled(invocation, () => this._wm.setWindowFullscreen(id, fullscreen));
    }

    SetWindowAboveAsync(params, invocation) {
        const [id, above] = params;
        replyWhenSettled(invocation, () => this._wm.setWindowAbove(id, above));
    }

    SetWindowOnAllWorkspacesAsync(params, invocation) {
        const [id, onAllWorkspaces] = params;
        replyWhenSettled(
            invocation, () => this._wm.setWindowOnAllWorkspaces(id, onAllWorkspaces));
    }

    RestackWindowAsync(params, invocation) {
        const [id, stacking] = params;
        replyWhenSettled(invocation, () => this._wm.restackWindow(id, stacking));
    }

    /* Asynchronous by necessity, not by preference.
     *
     * PointerManager.warpPointer() waits for the warp to actually land before
     * resolving (the warp is asynchronous inside Mutter - see pointer.js), so
     * this cannot be a plain synchronous method returning a value. GJS's
     * Gio.DBusExportedObject dispatches a method named `<Name>Async` with the
     * raw parameter tuple and the invocation, leaving us to reply ourselves.
     *
     * The `Async` suffix is the wrapper's convention and does NOT appear in the
     * interface XML - the method is `WarpPointer` on the bus. A daemon calling
     * `WarpPointerAsync` over D-Bus would get "no such method".
     */
    WarpPointerAsync(params, invocation) {
        const [x, y] = params;
        replyWhenSettled(
            invocation,
            () => this._pointer.warpPointer(x, y),
            position => new GLib.Variant('(ii)', [position.x, position.y])
        );
    }

    GetPointer() {
        const {x, y} = this._pointer.getPointer();
        return [x, y];
    }

    /* Synchronous, and it must stay that way.
     *
     * There is nothing to wait for - the answer is a read of live compositor
     * state - and waiting would actively make it worse: the value describes one
     * instant, and the user's hand is on the mouse. See
     * WindowManager.getWindowAtPointer().
     */
    GetWindowAtPointer() {
        const {found, id} = this._wm.getWindowAtPointer();
        return [found, id];
    }
}
