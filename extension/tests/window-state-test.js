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

import {parseStacking} from '../windows.js';

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

print('');
print(failures === 0 ? 'All window-state tests passed.' : `${failures} check(s) failed.`);
if (failures > 0)
    imports.system.exit(1);
