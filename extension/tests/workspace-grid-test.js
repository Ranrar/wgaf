#!/usr/bin/env -S gjs -m
/* workspace-grid-test.js
 *
 * Tests for resolveGrid() in windows.js, run with plain gjs.
 *
 * ---------------------------------------------------------------------------
 * WHY windows.js CAN BE IMPORTED HERE AT ALL
 * ---------------------------------------------------------------------------
 * It reaches for `global.display` and `global.workspace_manager`, which exist
 * only inside GNOME Shell - but only from *inside* the WindowManager class,
 * never at module top level. So importing the module is safe outside a Shell,
 * and any function that does not touch those singletons can be tested. That is
 * the line worth remembering when adding to this file: not "is it in
 * extension/", but "does it need the compositor".
 *
 * ---------------------------------------------------------------------------
 * WHAT THIS IS PROTECTING
 * ---------------------------------------------------------------------------
 * `wgaf workspace layout` printed `1 rows x -1 columns` on a real desktop -
 * caught by examples/desktop-layout.sh on its first live run. Mutter reports
 * -1 for "as many columns as needed" and documents it nowhere, so the sentinel
 * reached the user as if it were a count. The grid exists to answer "which
 * workspace is to the right"; -1 cannot answer anything.
 *
 * Run it with:
 *
 *     make -C extension test
 */

import {resolveGrid} from '../windows.js';

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

print('the layout GNOME actually reports');
// The measured case, and the reason this function exists: rows=1, columns=-1,
// constant across workspace counts on GNOME Shell 50.1 / Mutter 18.
check('one workspace', resolveGrid(1, -1, 1), {rows: 1, columns: 1});
check('two workspaces', resolveGrid(1, -1, 2), {rows: 1, columns: 2});
check('four workspaces', resolveGrid(1, -1, 4), {rows: 1, columns: 4});

print('a sentinel never reaches the caller');
for (const [rows, columns, n] of [[1, -1, 3], [-1, 2, 4], [-1, -1, 5], [0, 0, 2]]) {
    const grid = resolveGrid(rows, columns, n);
    if (grid.rows > 0 && grid.columns > 0) {
        print(`  ok    (${rows}, ${columns}) with ${n} workspaces -> (${grid.rows}, ${grid.columns})`);
    } else {
        print(`  FAIL  (${rows}, ${columns}) with ${n} workspaces -> (${grid.rows}, ${grid.columns})`);
        failures++;
    }
}

print('a grid Mutter states outright is passed through untouched');
// Set via override_workspace_layout. wgaf must not second-guess a real answer.
check('2x3 stays 2x3', resolveGrid(2, 3, 6), {rows: 2, columns: 3});
check('even when it does not divide evenly', resolveGrid(2, 3, 4), {rows: 2, columns: 3});

print('the derived side rounds up rather than losing a workspace');
// 5 workspaces in 2 rows needs 3 columns; 2 would only hold 4, and the missing
// one is exactly the workspace a navigation script would fail to reach.
check('5 in 2 rows needs 3 columns', resolveGrid(2, -1, 5), {rows: 2, columns: 3});
check('5 in 2 columns needs 3 rows', resolveGrid(-1, 2, 5), {rows: 3, columns: 2});

print('the grid always holds every workspace');
for (let n = 1; n <= 12; n++) {
    for (const [rows, columns] of [[1, -1], [-1, 1], [2, -1], [-1, 3], [-1, -1]]) {
        const grid = resolveGrid(rows, columns, n);
        if (grid.rows * grid.columns < n) {
            print(`  FAIL  ${n} workspaces do not fit in ${grid.rows}x${grid.columns} (from ${rows}, ${columns})`);
            failures++;
        }
    }
}
print(`  ok    every combination up to 12 workspaces has room for all of them`);

print('a session reporting no workspaces still describes a usable grid');
// Should not happen, but dividing by a zero this function returned would be a
// fault in wgaf rather than in the compositor that reported the odd number.
check('zero workspaces', resolveGrid(-1, -1, 0), {rows: 1, columns: 1});

print('');
print(failures === 0 ? 'All workspace-grid tests passed.' : `${failures} check(s) failed.`);
if (failures > 0)
    imports.system.exit(1);
