/* pointer.js
 *
 * Pointer position and absolute pointer movement for the wgaf D-Bus bridge.
 *
 * Separate from windows.js on purpose: that file is about Meta.Window, and
 * pointer state is a different concern with a different backing API (Clutter's
 * seat, not Mutter's window manager). Like windows.js this module is
 * D-Bus-agnostic - it deals in plain JS numbers and Promises, and all GVariant
 * marshaling lives in dbusInterface.js.
 *
 * ---------------------------------------------------------------------------
 * WHY THE POINTER MOVES FROM IN HERE AND NOT FROM `uinput`
 * ---------------------------------------------------------------------------
 * The daemon's input backend synthesizes *relative* motion (REL_X/REL_Y) through
 * a virtual `uinput` device. Relative motion cannot express "put the pointer at
 * (x, y)": libinput applies pointer acceleration to it, so the distance actually
 * travelled is not the distance requested. Absolute positioning is a geometry
 * problem, and the compositor is the only thing that knows the geometry.
 *
 * Clutter's seat exposes exactly that, and the Shell is inside the compositor
 * process, so the extension can ask for it directly. This respects the
 * compositor's security model rather than working around it: nothing here
 * bypasses Wayland, it asks Mutter to do something Mutter is willing to do.
 *
 * Note the Wayland `wp_pointer_warp_v1` protocol is NOT an alternative here. It
 * is a *client* protocol, and Mutter only honours it while an implicit grab is
 * held - a headless daemon has neither a surface nor a grab.
 *
 * ---------------------------------------------------------------------------
 * MEASURED BEHAVIOUR - GNOME Shell 50.1 / Mutter 18, verified 2026-08-02
 * ---------------------------------------------------------------------------
 * Every claim below was measured against a live session, not inferred:
 *
 * - `warp_pointer` is pixel-exact. The pointer lands on the requested
 *   coordinate with no acceleration, rounding or drift, including on a
 *   90-degree-rotated monitor and across monitor boundaries.
 * - The warp is ASYNCHRONOUS. `global.get_pointer()` called immediately after
 *   `warp_pointer()` still reports the OLD position; the new one is visible
 *   within about 5ms. This is why warpPointer() below returns a Promise and
 *   confirms - see the comment on CONFIRM_TIMEOUT_MS.
 * - Mutter CLAMPS a coordinate that is not on any monitor, silently: no error,
 *   no signal, the pointer simply ends up somewhere else. The clamp is relative
 *   to the monitor the pointer currently occupies, so it is not even "nearest
 *   valid point". **Bounds checking is therefore the daemon's job, and it must
 *   happen before calling WarpPointer** - by the time Mutter has the coordinate,
 *   the information that it was invalid is gone. Nothing in this file rejects
 *   an off-screen coordinate, deliberately: the extension is not the authority
 *   on the monitor layout, the daemon is (it reads
 *   org.gnome.Mutter.DisplayConfig directly).
 * - A warp is a TELEPORT, not a motion path. Clients receive one motion event
 *   plus the ordinary enter/leave crossings, with no intermediate positions.
 *   An application watching for a drag-like sequence will not see one.
 * - Clients observe the warp as ordinary pointer motion, and the coordinate
 *   space is exactly Meta.Window.get_frame_rect()'s - a client's own reported
 *   position equals (warp target - frame rect origin) exactly. Measured on
 *   scale-1.0 monitors only; fractional scaling is untested.
 */

import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';

/* How often to re-check whether the warp has landed, in milliseconds.
 *
 * The measured settle time is under 5ms, so this polls fast enough that a
 * successful warp confirms on the first or second tick and the D-Bus round trip
 * dominates the cost.
 */
const POLL_INTERVAL_MS = 2;

/* How long to wait for the pointer to reach the requested position before
 * giving up and reporting wherever it actually is.
 *
 * This is a bound on a wait, not a tuning knob, and it is NOT an error path:
 * expiring is a legitimate outcome. If the user moves a real mouse while the
 * warp is in flight, the pointer genuinely will not be at the requested
 * coordinate, and no amount of waiting changes that. Reporting the true
 * position is more useful than either lying or failing.
 *
 * 100ms against a ~5ms need is deliberately generous. The cost of it being too
 * long is a slow reply in a case that does not happen; the cost of it being too
 * short is a wrong answer, which is worse.
 */
const CONFIRM_TIMEOUT_MS = 100;

/** Pointer queries and absolute pointer movement.
 *
 * Holds no state: every method reads live compositor state. There is nothing to
 * tear down, which is why there is no destroy() here and extension.js does not
 * call one.
 */
export class PointerManager {
    /** The pointer's current position in global logical pixels.
     *
     * `global.get_pointer()` returns [x, y, modifierState]; the modifier state
     * is deliberately dropped. Keyboard modifier state is not pointer position,
     * a caller asking where the pointer is has not asked what keys are held,
     * and exposing it here would make this method a second, worse answer to a
     * question `wgaf key` already owns.
     */
    getPointer() {
        const [x, y] = global.get_pointer();
        return {x, y};
    }

    /** Move the pointer to an absolute position, resolving once it has landed.
     *
     * Resolves with the position the pointer actually reached, which is
     * normally the requested one. It differs only if the warp was clamped (an
     * off-screen coordinate the daemon should have rejected before calling) or
     * if something else moved the pointer while the warp was in flight.
     *
     * **Why this confirms rather than returning immediately.** The warp is
     * asynchronous, so a caller that returned straight away would let
     * `wgaf mouse move-to X Y` be followed by a click that fires at the OLD
     * position - the automation equivalent of a check-then-act race, and the
     * same shape of defect as the focus hazard already recorded against the
     * input backend. Confirming here rather than in the daemon keeps one D-Bus
     * round trip as one round trip, and this is the process that holds the
     * state being waited on.
     *
     * The wait is a GLib timeout rather than a busy loop because this runs on
     * the compositor's main loop: spinning here would stutter the whole desktop
     * for the duration, on every single pointer move.
     */
    warpPointer(x, y) {
        const seat = Clutter.get_default_backend().get_default_seat();
        seat.warp_pointer(x, y);

        return new Promise(resolve => {
            let waited = 0;
            GLib.timeout_add(GLib.PRIORITY_DEFAULT, POLL_INTERVAL_MS, () => {
                const [currentX, currentY] = global.get_pointer();
                waited += POLL_INTERVAL_MS;

                const arrived = currentX === x && currentY === y;
                if (arrived || waited >= CONFIRM_TIMEOUT_MS) {
                    resolve({x: currentX, y: currentY});
                    return GLib.SOURCE_REMOVE;
                }
                return GLib.SOURCE_CONTINUE;
            });
        });
    }
}
