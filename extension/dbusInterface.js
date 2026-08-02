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

import {WindowNotFoundError} from './windows.js';

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
 * (i), focused (b), maximized (b). `id` is Meta.Window's stable sequence
 * number, not its (Wayland-unsafe) X11 XID - see windows.js.
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

/** Map a JS exception thrown out of WindowManager into a named D-Bus error
 * where we recognize it, or pass it through unchanged otherwise. Unknown
 * exceptions still safely become a generic D-Bus error reply via
 * Gio.DBusExportedObject's own exception handling - they don't crash the
 * Shell process - but only errors we explicitly recognize get a stable,
 * documented D-Bus error name the daemon can match on.
 */
function translateError(error) {
    if (error instanceof WindowNotFoundError)
        return Gio.DBusError.new_for_dbus_error(DBusErrors.WINDOW_NOT_FOUND, error.message);
    return error;
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

    FocusWindow(id) {
        try {
            this._wm.focusWindow(id);
        } catch (e) {
            throw translateError(e);
        }
    }

    MoveWindow(id, x, y) {
        try {
            this._wm.moveWindow(id, x, y);
        } catch (e) {
            throw translateError(e);
        }
    }

    ResizeWindow(id, width, height) {
        try {
            this._wm.resizeWindow(id, width, height);
        } catch (e) {
            throw translateError(e);
        }
    }

    CloseWindow(id) {
        try {
            this._wm.closeWindow(id);
        } catch (e) {
            throw translateError(e);
        }
    }

    GetWorkspaces() {
        return this._wm.getWorkspaces().map(workspaceRecordToVariantDict);
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
        this._pointer.warpPointer(x, y).then(position => {
            invocation.return_value(new GLib.Variant('(ii)', [position.x, position.y]));
        }).catch(error => {
            invocation.return_error_literal(
                Gio.DBusError,
                Gio.DBusError.FAILED,
                `wgaf: could not move the pointer: ${error.message}`
            );
        });
    }

    GetPointer() {
        const {x, y} = this._pointer.getPointer();
        return [x, y];
    }
}
