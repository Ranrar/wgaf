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
import {PointerManager} from './pointer.js';
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
/* These names must match what the daemon actually provides. If you rename one
 * here without renaming it in the daemon (or the other way round), nothing
 * will report an error: the extension simply reads nothing back, assumes wgaf
 * is not running automation, and never takes the emergency key - so the panic
 * button quietly stops existing.
 *
 * A test guards the pair. If it fails, change these to match the daemon rather
 * than changing the test.
 */
const DAEMON_BUS_NAME = 'org.wgaf.Daemon';
const DAEMON_OBJECT_PATH = '/org/wgaf/Daemon';
const DAEMON_INTERFACE_XML = `
<node>
  <interface name="org.wgaf.Daemon1">
    <method name="Stop"/>
    <property name="InputDeviceActive" type="b" access="read"/>
  </interface>
</node>`;

const WgafDaemonProxy = Gio.DBusProxy.makeProxyWrapper(DAEMON_INTERFACE_XML);

/* GSettings key holding the kill switch shortcut. Keyboard shortcuts are
 * string arrays, so this is type "as" - see schemas/.
 */
const KILL_SWITCH_KEY = 'kill-switch';

/* wgaf's own virtual keyboard, as the kernel advertises it.
 *
 * These must match `VENDOR_ID`/`PRODUCT_ID` in the daemon's
 * input/device.rs. They identify keystrokes wgaf synthesized itself, which
 * the emergency key must ignore - otherwise a script that presses Escape to
 * dismiss a dialog stops the very run that issued it.
 *
 * Deliberately not the device *name*: that is configurable (config.toml's
 * input_device_name), so a renamed device would silently switch the emergency
 * key off. The vendor and product IDs are fixed in the daemon's source.
 *
 * Clutter reports both as integers - confirmed against Mutter 18, where
 * get_vendor_id() and get_product_id() are typed `int`, not hex strings.
 */
const WGAF_VENDOR_ID = 0x57ae;
const WGAF_PRODUCT_ID = 0x0001;

export default class WgafExtension extends Extension {
    enable() {
        this._windowManager = new WindowManager();

        // Holds no state and connects no signals, so unlike WindowManager it
        // needs no teardown in disable() - dropping the reference is enough.
        this._pointerManager = new PointerManager();

        this._dbusInterface = new WgafDBusInterface(this._windowManager, this._pointerManager);
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

    /* Prepares the emergency key, without yet taking it.
     *
     * The shortcut is a *grab*: while it is registered, the compositor consumes
     * that key before any application sees it. Registering it for the whole
     * session therefore took Escape away from the entire desktop - dialogs
     * would not close, vim never left insert mode - even while the daemon was
     * idle or not running at all.
     *
     * So it is held only while wgaf can actually type, which the daemon
     * reports as org.wgaf.Daemon1.InputDeviceActive. Between runs the key
     * belongs to your applications, which is nearly all of the time.
     *
     * The proxy is created once, here, rather than when the key is pressed. It
     * survives the daemon starting later or restarting, and it keeps the
     * emergency path free of setup work.
     */
    _enableKillSwitch() {
        this._killSwitchArmed = false;

        this._daemonProxy = new WgafDaemonProxy(
            Gio.DBus.session,
            DAEMON_BUS_NAME,
            DAEMON_OBJECT_PATH,
            (proxy, error) => {
                if (error) {
                    console.error(`wgaf: could not reach the wgaf daemon: ${error.message}`);
                    return;
                }
                this._syncKillSwitch();
            }
        );

        this._daemonPropertiesId = this._daemonProxy.connect(
            'g-properties-changed',
            () => this._syncKillSwitch()
        );

        /* A daemon that crashes mid-run emits no closing property change, so
         * without this the grab would outlive it and Escape would stay
         * captured by nothing at all.
         */
        this._daemonWatchId = Gio.bus_watch_name(
            Gio.BusType.SESSION,
            DAEMON_BUS_NAME,
            Gio.BusNameWatcherFlags.NONE,
            () => this._syncKillSwitch(),
            () => this._disarmKillSwitch()
        );
    }

    /* Holds the emergency key exactly while the daemon can type. */
    _syncKillSwitch() {
        // `?? false` covers the daemon being absent, and an older daemon that
        // does not publish the property at all. Both mean "cannot type now".
        const canType = this._daemonProxy?.InputDeviceActive ?? false;

        if (canType)
            this._armKillSwitch();
        else
            this._disarmKillSwitch();
    }

    /* Shell.ActionMode.ALL on purpose: the key has to work while the overview
     * is open, while a menu has grabbed the pointer, and in every other state
     * a runaway script can leave the desktop in. A brake that only works when
     * things are calm is not a brake.
     */
    _armKillSwitch() {
        if (this._killSwitchArmed)
            return;

        const action = Main.wm.addKeybinding(
            KILL_SWITCH_KEY,
            this.getSettings(),
            Meta.KeyBindingFlags.IGNORE_AUTOREPEAT,
            Shell.ActionMode.ALL,
            (display, window, event) => this._onKillSwitchPressed(event)
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
            return;
        }

        this._killSwitchArmed = true;
    }

    _disarmKillSwitch() {
        if (!this._killSwitchArmed)
            return;

        Main.wm.removeKeybinding(KILL_SWITCH_KEY);
        this._killSwitchArmed = false;
    }

    /* Decides whether a press of the emergency key was the user's or wgaf's.
     *
     * wgaf types through a virtual keyboard of its own, and the compositor
     * hands this handler the event that triggered the shortcut - including
     * which device sent it. A key wgaf synthesized is ignored, so a script
     * that presses Escape to dismiss a dialog no longer stops itself.
     *
     * Note that the key is consumed either way: a matched shortcut is taken
     * before this runs, and there is no way to pass it on. So while a run is
     * in progress, a synthesized Escape reaches nothing - it neither stops
     * wgaf nor closes the dialog. Dismiss dialogs through the accessibility
     * layer instead.
     *
     * An event with no identifiable device counts as the user's. If wgaf
     * cannot prove a keystroke was its own, the only safe reading is that
     * somebody is asking it to stop.
     */
    _onKillSwitchPressed(event) {
        const device = event?.get_source_device?.();

        if (device &&
            device.get_vendor_id() === WGAF_VENDOR_ID &&
            device.get_product_id() === WGAF_PRODUCT_ID) {
            // Debug rather than silence: an emergency key that appears to do
            // nothing is precisely the thing that must leave a trace.
            console.debug('wgaf: ignoring an emergency key press from wgaf\'s own virtual keyboard');
            return;
        }

        this._stopDaemon();
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
        // handed back here - otherwise the emergency key stays captured by a
        // disabled extension and reaches nothing at all.
        this._disarmKillSwitch();

        if (this._daemonWatchId) {
            Gio.bus_unwatch_name(this._daemonWatchId);
            this._daemonWatchId = null;
        }

        if (this._daemonPropertiesId && this._daemonProxy) {
            this._daemonProxy.disconnect(this._daemonPropertiesId);
            this._daemonPropertiesId = null;
        }

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

        this._pointerManager = null;

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
