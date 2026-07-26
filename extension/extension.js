/* extension.js
 *
 * Lifecycle entry point for the wgaf window bridge extension. Wires the
 * WindowManager (windows.js, pure Mutter/Meta logic) to a D-Bus-exported
 * object (dbusInterface.js) and owns the bus name.
 *
 * Verified against GNOME Shell 50.1 (ESM-style extensions: a default-exported
 * class extending Extension from resource:///org/gnome/shell/extensions/extension.js,
 * not the pre-GNOME-45 init()/imports.misc.extensionUtils style).
 */

import Gio from 'gi://Gio';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import {WindowManager} from './windows.js';
import {
    WgafDBusInterface,
    DBUS_INTERFACE_XML,
    DBUS_BUS_NAME,
    DBUS_OBJECT_PATH,
    windowCreatedSignalVariant,
    windowClosedSignalVariant,
    windowFocusChangedSignalVariant,
} from './dbusInterface.js';

export default class WgafExtension extends Extension {
    enable() {
        this._windowManager = new WindowManager();

        this._dbusInterface = new WgafDBusInterface(this._windowManager);
        this._dbusImpl = Gio.DBusExportedObject.wrapJSObject(DBUS_INTERFACE_XML, this._dbusInterface);
        this._dbusImpl.export(Gio.DBus.session, DBUS_OBJECT_PATH);

        // Own the well-known bus name so daemon clients can find us by name
        // rather than only by object path. No name-lost/acquired callbacks
        // are needed here: we already exported the object above, and losing
        // the name (e.g. because another instance already owns it) doesn't
        // need special handling beyond what bus_unown_name() in disable()
        // undoes.
        this._ownerId = Gio.bus_own_name(
            Gio.BusType.SESSION,
            DBUS_BUS_NAME,
            Gio.BusNameOwnerFlags.NONE,
            null,
            null,
            null
        );

        // ADDED: wire Mutter-side window/focus events (delivered by
        // WindowManager as plain JS records/ids - see windows.js) to actual
        // D-Bus signal emission. GVariant construction is D-Bus plumbing, so
        // it happens here via dbusInterface.js's signal-variant helpers,
        // not inside windows.js itself.
        this._windowManager.connectSignals((signalName, payload) => {
            if (!this._dbusImpl)
                return;
            switch (signalName) {
            case 'WindowCreated':
                this._dbusImpl.emit_signal('WindowCreated', windowCreatedSignalVariant(payload));
                break;
            case 'WindowClosed':
                this._dbusImpl.emit_signal('WindowClosed', windowClosedSignalVariant(payload));
                break;
            case 'WindowFocusChanged':
                this._dbusImpl.emit_signal('WindowFocusChanged', windowFocusChangedSignalVariant(payload));
                break;
            }
        });
    }

    disable() {
        // FIXED: every signal connected in enable() must be torn down here.
        // WindowManager.destroy() disconnects both the global.display
        // signals (window-created, notify::focus-window) and every
        // per-window `unmanaging` handler it accumulated - without this,
        // re-enabling the extension (or a Shell restart under Xorg/nested
        // testing) would leak handlers onto stale Meta.Window objects.
        if (this._windowManager) {
            this._windowManager.destroy();
            this._windowManager = null;
        }

        if (this._ownerId) {
            Gio.bus_unown_name(this._ownerId);
            this._ownerId = null;
        }

        if (this._dbusImpl) {
            this._dbusImpl.unexport();
            this._dbusImpl = null;
        }

        this._dbusInterface = null;
    }
}
