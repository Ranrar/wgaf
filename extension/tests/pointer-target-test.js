#!/usr/bin/env -S gjs -m
/* pointer-target-test.js
 *
 * Tests for topmostAt() in windows.js, run with plain gjs.
 *
 * ---------------------------------------------------------------------------
 * WHY THIS ONE IS WORTH UNIT-TESTING
 * ---------------------------------------------------------------------------
 * It is the arithmetic behind wgaf's mouse-targeting guard: `wgaf mouse click
 * --window <id>` refuses to click unless this function names that window. Two
 * kinds of mistake in here are invisible from outside:
 *
 *  - **Reading the stack the wrong way up.** sort_windows_by_stacking() returns
 *    bottom-first, so the topmost match is the LAST one. Scanning forward
 *    instead would attribute a click to the window UNDERNEATH the one that will
 *    receive it - and the guard would happily approve a click going somewhere
 *    the caller did not target, which is the exact hazard it exists to stop.
 *  - **Inclusive edges.** Two windows touching along an edge would both claim
 *    the boundary pixel, so the answer would depend on scan order.
 *
 * Neither shows up as a crash or a wrong-looking output; both show up as a
 * click in the wrong window, occasionally.
 *
 * windows.js is importable outside a Shell because it only reaches for
 * `global.*` from inside the WindowManager class - see workspace-grid-test.js.
 *
 * Run it with:
 *
 *     make -C extension test
 */

import {topmostAt} from '../windows.js';

let failures = 0;

function check(description, actual, expected) {
    if (actual === expected) {
        print(`  ok    ${description}`);
    } else {
        print(`  FAIL  ${description}: expected ${expected}, got ${actual}`);
        failures++;
    }
}

const window = (id, x, y, width, height) => ({id, rect: {x, y, width, height}});

print('a point over one window');
const single = [window(1, 100, 100, 200, 150)];
check('inside', topmostAt(single, 150, 150), 1);
check('top-left corner is inside', topmostAt(single, 100, 100), 1);
check('left of it', topmostAt(single, 99, 150), null);
check('above it', topmostAt(single, 150, 99), null);
check('right of it', topmostAt(single, 300, 150), null);
check('below it', topmostAt(single, 150, 250), null);

print('empty desktop');
check('no windows at all', topmostAt([], 150, 150), null);

print('overlapping windows resolve to the topmost');
// Bottom-first, as sort_windows_by_stacking() returns them: 3 is on top.
const stacked = [
    window(1, 0, 0, 400, 400),
    window(2, 100, 100, 200, 200),
    window(3, 150, 150, 100, 100),
];
check('where all three overlap', topmostAt(stacked, 200, 200), 3);
check('where the lower two overlap', topmostAt(stacked, 120, 120), 2);
check('where only the bottom one is', topmostAt(stacked, 10, 10), 1);
check('outside all of them', topmostAt(stacked, 500, 500), null);

print('a window fully covered by another is never the answer');
const covered = [window(1, 100, 100, 50, 50), window(2, 0, 0, 400, 400)];
check('the coverer wins everywhere it overlaps', topmostAt(covered, 120, 120), 2);

print('edges are half-open, so touching windows do not both claim a pixel');
// 1 covers x 0..99, 2 covers x 100..199. The shared boundary is 100, and it
// belongs to exactly one of them.
const adjacent = [window(1, 0, 0, 100, 100), window(2, 100, 0, 100, 100)];
check('the last column of the left window', topmostAt(adjacent, 99, 50), 1);
check('the boundary belongs to the right window', topmostAt(adjacent, 100, 50), 2);
check('past the right window', topmostAt(adjacent, 200, 50), null);

print('negative coordinates - a monitor left of or above the primary');
const offPrimary = [window(7, -1920, -100, 1920, 1080)];
check('inside a negative-origin window', topmostAt(offPrimary, -1000, 500), 7);
check('outside it', topmostAt(offPrimary, 10, 500), null);

print('zero-sized rectangles claim nothing');
// A window mid-creation reports 0x0 - measured on WindowCreated, see the
// proxy's doc comment. It must not swallow the point at its own origin.
check('a 0x0 window at the point', topmostAt([window(9, 50, 50, 0, 0)], 50, 50), null);

print('');
print(failures === 0 ? 'All pointer-target tests passed.' : `${failures} check(s) failed.`);
if (failures > 0)
    imports.system.exit(1);
