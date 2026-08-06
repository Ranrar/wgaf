/* confirm.js
 *
 * Waiting for a compositor state change to actually take effect, inside the
 * Shell process that holds the state.
 *
 * ---------------------------------------------------------------------------
 * WHY THIS EXISTS
 * ---------------------------------------------------------------------------
 * Asking Mutter to do something and asking whether it has been done are two
 * different questions, and several of Mutter's methods answer the first one
 * immediately while the second stays false for a few milliseconds. A bridge
 * method that returns as soon as the request has been sent therefore reports
 * success before the change is readable, and the caller's very next command
 * acts on the old state - a check-then-act race introduced by the bridge
 * itself.
 *
 * pointer.js met this first: `warp_pointer()` is asynchronous, and
 * `global.get_pointer()` immediately afterwards still reports the old position.
 * It confirms by polling, and this module is that same wait generalized, so
 * every operation that changes compositor state can use one implementation
 * instead of each growing its own timeout constant.
 *
 * Confirming HERE rather than in the daemon is deliberate: this is the process
 * that holds the state being waited on, so one D-Bus round trip stays one round
 * trip. A daemon-side poll would be a round trip per tick.
 *
 * ---------------------------------------------------------------------------
 * THIS IS A BOUND ON A WAIT, NOT A GUESS AT A DURATION
 * ---------------------------------------------------------------------------
 * confirmSettled() resolves the instant the state matches, so a fast operation
 * costs one tick regardless of the timeout. The timeout only decides how long
 * to keep looking before reporting that it never happened - which is a real
 * outcome for several of these operations, not an error condition invented
 * here. Callers are told which of the two occurred and decide what it means.
 */

import GLib from 'gi://GLib';

/* How often to re-read the state, in milliseconds.
 *
 * Matches pointer.js's interval, and for the same reason: the settle times
 * measured on this compositor are single-digit milliseconds, so a successful
 * operation confirms on the first or second tick and the D-Bus round trip
 * dominates the cost.
 */
const POLL_INTERVAL_MS = 2;

/* How long to keep looking before reporting that the change never became
 * readable.
 *
 * Generous against a single-digit-millisecond need, on the same trade
 * pointer.js documents: too long costs a slow reply in a case that does not
 * happen, too short costs a wrong answer.
 */
const CONFIRM_TIMEOUT_MS = 100;

/**
 * Poll `read()` until `isSettled(value)` holds, or the timeout expires.
 *
 * Resolves with `{value, confirmed}` - the last value read, and whether it
 * satisfied `isSettled`. Never rejects: "it did not happen" is reported as
 * `confirmed: false` rather than thrown, because whether that is a failure
 * depends on the operation and belongs to the caller. windows.js turns it into
 * an OperationNotAppliedError for workspace operations; pointer.js's equivalent
 * treats it as a legitimate outcome and reports the true position.
 *
 * Checks once before waiting at all, so an operation that has already taken
 * effect by the time this is called costs nothing.
 *
 * The wait is a GLib timeout rather than a busy loop because this runs on the
 * compositor's main loop - spinning here would stutter the whole desktop for
 * the duration, on every operation.
 */
export function confirmSettled(read, isSettled, {
    timeoutMs = CONFIRM_TIMEOUT_MS,
    pollMs = POLL_INTERVAL_MS,
} = {}) {
    const initial = read();
    if (isSettled(initial))
        return Promise.resolve({value: initial, confirmed: true});

    return new Promise(resolve => {
        let waited = 0;
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, pollMs, () => {
            const value = read();
            waited += pollMs;

            if (isSettled(value)) {
                resolve({value, confirmed: true});
                return GLib.SOURCE_REMOVE;
            }
            if (waited >= timeoutMs) {
                resolve({value, confirmed: false});
                return GLib.SOURCE_REMOVE;
            }
            return GLib.SOURCE_CONTINUE;
        });
    });
}
