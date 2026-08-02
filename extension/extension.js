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
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

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

/* The wgaf daemon's own D-Bus interface, of which only the kill switch is
 * used here. This is the one place the extension is a *client* of the daemon
 * rather than a service it calls into: everywhere else the daemon asks the
 * extension for windows, but a panic stop has to travel the other way.
 *
 * Only Stop() appears. Resuming is deliberately not on a shortcut — a key you
 * hit repeatedly during a failure must never restart the automation that
 * caused it, so coming back is always the `wgaf release` command, typed once.
 *
 * The bus name is wgaf's default. A daemon started with a customized
 * `bus_name` in its config.toml will not hear this shortcut; use `wgaf stop`.
 */
const DAEMON_BUS_NAME = 'org.wgaf.Daemon';
const DAEMON_OBJECT_PATH = '/org/wgaf/Daemon';
const DAEMON_INTERFACE_XML = `
<node>
  <interface name="org.wgaf.Daemon1">
    <method name="Stop"/>
  </interface>
</node>`;

const WgafDaemonProxy = Gio.DBusProxy.makeProxyWrapper(DAEMON_INTERFACE_XML);

/* GSettings key holding the kill switch shortcut. Keyboard shortcuts are
 * string arrays, so this is type "as" - see schemas/.
 */
const KILL_SWITCH_KEY = 'kill-switch';

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

        this._enableKillSwitch();
    }

    /* Installs the keyboard shortcut that stops wgaf's input automation.
     *
     * Shell.ActionMode.ALL on purpose: the shortcut has to work while the
     * overview is open, while a menu has grabbed the pointer, and in every
     * other state a runaway script can leave the desktop in. A brake that only
     * works when things are calm is not a brake.
     *
     * The proxy is created once, here, rather than when the shortcut is
     * pressed. It survives the daemon starting later or restarting, and it
     * keeps the emergency path free of setup work.
     */
    _enableKillSwitch() {
        this._daemonProxy = new WgafDaemonProxy(
            Gio.DBus.session,
            DAEMON_BUS_NAME,
            DAEMON_OBJECT_PATH,
            (proxy, error) => {
                if (error)
                    console.error(`wgaf: could not reach the wgaf daemon: ${error.message}`);
            }
        );

        const action = Main.wm.addKeybinding(
            KILL_SWITCH_KEY,
            this.getSettings(),
            Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
            Shell.ActionMode.ALL,
            () => this._stopDaemon()
        );

        /* A shortcut that silently failed to register is worse than none:
         * you would only find out at the moment you needed it. This does not
         * catch a key another shortcut has already claimed - the compositor
         * resolves that by letting whichever was registered last win, without
         * telling anyone.
         */
        if (action === Meta.KeyBindingAction.NONE) {
            console.error('wgaf: the kill switch shortcut could not be registered. ' +
                'Use `wgaf stop` instead, or choose a different shortcut with ' +
                '`gsettings set org.gnome.shell.extensions.wgaf kill-switch`.');
        }
    }

    /* Asks the daemon to stop synthesizing input, and says so either way.
     *
     * A method call rather than a signal, so that an unreachable daemon fails
     * where the user can see it: a panic button that has quietly stopped
     * working is worse than no panic button at all.
     */
    _stopDaemon() {
        if (!this._daemonProxy)
            return;

        this._daemonProxy.StopAsync()
            .then(() => {
                Main.notify(
                    'wgaf input stopped',
                    'No further keyboard or mouse automation will run. Run `wgaf release` to allow it again.'
                );
            })
            .catch(error => {
                Main.notifyError(
                    'wgaf could not be stopped',
                    `The wgaf daemon did not answer: ${error.message}`
                );
            });
    }

    disable() {
        // The shortcut is the Shell's while it is installed, so it has to be
        // handed back here - otherwise Ctrl+Alt+Escape stays captured by a
        // disabled extension and reaches nothing at all.
        Main.wm.removeKeybinding(KILL_SWITCH_KEY);
        this._daemonProxy = null;

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
