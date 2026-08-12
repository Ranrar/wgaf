#!/usr/bin/env -S gjs -m
/* window-state-test.js
 *
 * Tests for parseStacking() in windows.js - the argument parsing behind
 * `wgaf window raise` / `lower` - run with plain gjs.
 *
 * ---------------------------------------------------------------------------
 * WHY ONLY THE PARSING
 * ---------------------------------------------------------------------------
 * The operations themselves call Meta.Window methods on a real window inside a
 * running compositor, and there is no way to stand one up for a test - see the
 * note in workspace-grid-test.js about where that line falls. What can be
 * pulled out and tested is the decision an operation makes *before* it touches
 * Mutter.
 *
 * ---------------------------------------------------------------------------
 * WHAT THIS IS PROTECTING
 * ---------------------------------------------------------------------------
 * The failure being guarded against is a silent one: an unrecognized value
 * defaulting to something rather than being rejected, so a typo moves a window
 * in a direction nobody asked for and reports success. Every case below
 * therefore also asserts that nothing is guessed.
 *
 * This file also used to cover a maximize-direction parser. That went with
 * `--direction` itself, once Mutter 18 was measured to offer no per-axis
 * maximize at all - see setWindowMaximized() in windows.js.
 *
 * Run it with:
 *
 *     make -C extension test
 */

import {parseStacking, windowTypeName} from '../windows.js';

let failures = 0;

function check(description, actual, expected) {
    const a = JSON.stringify(actual);
    const e = JSON.stringify(expected);
    if (a === e) {
        print(`  ok    ${description}`);
    } else {
        print(`  FAIL  ${description}: expected ${e}, got ${a}`);
        failures++;
    }
}

function checkThrows(description, fn) {
    try {
        const value = fn();
        print(`  FAIL  ${description}: returned ${JSON.stringify(value)} instead of throwing`);
        failures++;
    } catch (e) {
        print(`  ok    ${description}`);
    }
}

print('restacking goes one of two ways and nowhere else');
check('raise', parseStacking('raise'), 'raise');
check('lower', parseStacking('lower'), 'lower');

print('a direction is never guessed');
// Case matters, and so does everything else: a value that is nearly right must
// be refused rather than quietly resolved to whichever way looks closest.
for (const bad of ['Raise', 'RAISE', 'top', 'up', 'down', 'bottom', '', ' raise'])
    checkThrows(`'${bad}' is rejected`, () => parseStacking(bad));

print('a missing direction is refused rather than defaulted');
checkThrows('undefined', () => parseStacking(undefined));
checkThrows('null', () => parseStacking(null));

print('the rejection says what was expected');
// A caller talking to the extension directly gets this message and nothing
// else, so it has to name the alternatives rather than only the mistake.
try {
    parseStacking('sideways');
    print('  FAIL  no error raised');
    failures++;
} catch (e) {
    const names = ['raise', 'lower'].every(n => e.message.includes(n));
    if (names && e.message.includes('sideways')) {
        print('  ok    names both the bad value and the accepted ones');
    } else {
        print(`  FAIL  unhelpful message: ${e.message}`);
        failures++;
    }
}

print('window kinds are named, and an unknown one does not throw');
/* The order below is `Meta.WindowType`'s own, transcribed on 2026-08-11 from
 * `Object.keys(Meta.WindowType)` on Mutter 18. windows.js cannot import Meta
 * (a top-level import makes the file unloadable outside a Shell), so the names
 * are a literal there and this is the copy that would disagree if a future
 * Mutter inserted a value. A mismatch means every window reports the wrong
 * kind, silently and plausibly - `dialog` where `dock` was meant - which is why
 * the whole list is pinned rather than a sample of it. */
const META_WINDOW_TYPES = [
    'normal', 'desktop', 'dock', 'dialog', 'modal_dialog', 'toolbar', 'menu',
    'utility', 'splashscreen', 'dropdown_menu', 'popup_menu', 'tooltip',
    'notification', 'combo', 'dnd', 'override_other',
];
META_WINDOW_TYPES.forEach((name, value) =>
    check(`${value} is ${name}`, windowTypeName(value), name));

/* A value from a newer Mutter must not become an exception: it is thrown from
 * inside ListWindows, so losing the entire window list because one window is of
 * an unfamiliar kind would be far worse than not naming its kind. */
check('a value beyond the enum', windowTypeName(META_WINDOW_TYPES.length), 'unknown');
check('a wildly out-of-range value', windowTypeName(999), 'unknown');
check('a negative value', windowTypeName(-1), 'unknown');

print('');
print(failures === 0 ? 'All window-state tests passed.' : `${failures} check(s) failed.`);
if (failures > 0)
    imports.system.exit(1);
