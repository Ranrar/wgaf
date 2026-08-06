#!/usr/bin/env -S gjs -m
/* confirm-test.js
 *
 * Tests for confirm.js, run with plain gjs - no GNOME Shell, no compositor,
 * no session.
 *
 * ---------------------------------------------------------------------------
 * WHY THIS ONE FILE CAN BE TESTED AND THE REST OF THE EXTENSION CANNOT
 * ---------------------------------------------------------------------------
 * Everything else in extension/ reaches for `global.display`,
 * `global.workspace_manager` or Clutter's seat - objects that exist only inside
 * a running GNOME Shell process, which cannot be started for a test. confirm.js
 * imports GLib and nothing else, and takes the state it watches as two
 * callbacks. That makes it ordinary code, and ordinary code can be run.
 *
 * It is also the piece most worth covering. Every mutating method in the bridge
 * routes its "did that actually happen?" through here, so a bug in it does not
 * fail loudly - it reports success for changes that never landed, which is the
 * exact defect the module exists to prevent.
 *
 * Run it with:
 *
 *     make -C extension test
 *
 * Exits non-zero on the first failure, so it can gate anything.
 */

import GLib from 'gi://GLib';

import {confirmSettled} from '../confirm.js';

let failures = 0;

function check(description, actual, expected) {
    if (actual === expected) {
        print(`  ok    ${description}`);
    } else {
        print(`  FAIL  ${description}: expected ${expected}, got ${actual}`);
        failures++;
    }
}

function ok(description, condition) {
    check(description, Boolean(condition), true);
}

/* Each test is an async function so the assertions can be written after the
 * await, in the order they happen, rather than nested inside .then() chains.
 */
const tests = [];
function test(name, fn) {
    tests.push([name, fn]);
}

// ---------------------------------------------------------------------------

test('state that is already settled resolves without waiting', async () => {
    let reads = 0;
    const started = GLib.get_monotonic_time();

    const {value, confirmed} = await confirmSettled(
        () => {
            reads++;
            return 'done';
        },
        v => v === 'done',
        {timeoutMs: 5000, pollMs: 1000}
    );

    ok('confirmed', confirmed);
    check('reports the value it read', value, 'done');
    check('reads the state exactly once', reads, 1);
    // With a 1000ms poll interval, anything that waited for even one tick would
    // blow this. The check is that an operation Mutter applied synchronously
    // costs nothing at all, which is what makes it safe to route every
    // operation through here.
    ok(
        'does not wait for a poll tick',
        GLib.get_monotonic_time() - started < 500000
    );
});

test('state that settles later is confirmed when it does', async () => {
    let reads = 0;

    const {value, confirmed} = await confirmSettled(
        () => ++reads,
        v => v >= 3,
        {timeoutMs: 1000, pollMs: 2}
    );

    ok('confirmed', confirmed);
    check('reports the value that satisfied the condition', value, 3);
});

test('state that never settles reports not-confirmed rather than hanging', async () => {
    const {value, confirmed} = await confirmSettled(
        () => 'unchanged',
        v => v === 'something else',
        {timeoutMs: 30, pollMs: 2}
    );

    check('not confirmed', confirmed, false);
    // The last value actually read, not a placeholder. windows.js puts this in
    // the OperationNotApplied message, so "expected 2, got 0" is only possible
    // if the real reading survives the timeout.
    check('reports the state as it really is', value, 'unchanged');
});

test('the timeout bounds the wait', async () => {
    const started = GLib.get_monotonic_time();

    await confirmSettled(() => 0, () => false, {timeoutMs: 30, pollMs: 2});
    const elapsed = (GLib.get_monotonic_time() - started) / 1000;

    ok(`gives up after about 30ms, not sooner (took ${Math.round(elapsed)}ms)`, elapsed >= 25);
    // Generous upper bound: this shares a main loop with the other tests and
    // the point is that it terminates, not that it is punctual.
    ok(`does not overrun its timeout (took ${Math.round(elapsed)}ms)`, elapsed < 2000);
});

test('polling stops once the promise resolves', async () => {
    let reads = 0;

    await confirmSettled(() => ++reads, v => v >= 2, {timeoutMs: 1000, pollMs: 2});
    const atResolution = reads;

    // A GLib timeout source that returns SOURCE_CONTINUE after resolving would
    // keep running for the life of the process - inside GNOME Shell that means
    // polling compositor state forever, once per operation ever performed. It
    // would never fail a test that only checked the resolved value.
    await new Promise(resolve => {
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 50, () => {
            resolve();
            return GLib.SOURCE_REMOVE;
        });
    });

    check('no further reads after resolving', reads, atResolution);
});

test('a condition that throws does not leave the caller waiting forever', async () => {
    // windows.js reads live compositor state inside these callbacks, and a
    // window or workspace can vanish mid-poll. Whatever happens, the caller
    // must get an answer: a rejected promise is fine, a promise that never
    // settles is a D-Bus method that never replies.
    let settled = false;
    try {
        await confirmSettled(
            () => {
                throw new Error('state vanished');
            },
            () => true,
            {timeoutMs: 30, pollMs: 2}
        );
        settled = true;
    } catch {
        settled = true;
    }
    ok('the caller is told something either way', settled);
});

// ---------------------------------------------------------------------------

const loop = GLib.MainLoop.new(null, false);

(async () => {
    for (const [name, fn] of tests) {
        print(name);
        try {
            await fn();
        } catch (e) {
            print(`  FAIL  threw: ${e}`);
            failures++;
        }
    }

    print('');
    print(failures === 0
        ? `All ${tests.length} confirm.js tests passed.`
        : `${failures} check(s) failed.`);
    loop.quit();
})();

loop.run();

if (failures > 0)
    imports.system.exit(1);
