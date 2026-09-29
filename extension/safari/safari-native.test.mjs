// Rowel: the Safari native messaging port (safari-native.js), against a fake
// runtime whose sendNativeMessage the tests answer by hand. `npm run
// test:safari`; plain node --test, no dependencies.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const here = dirname(fileURLToPath(import.meta.url));
const SHIM = readFileSync(join(here, 'safari-native.js'), 'utf8');
const POLYFILL = readFileSync(join(here, '..', 'keepassxc-browser', 'common', 'browser-polyfill.min.js'), 'utf8');

const settle = async () => {
    for (let i = 0; i < 10; i++) {
        await new Promise((resolve) => setImmediate(resolve));
    }
};

const signal = (seq, action) => ({ seq, signal: JSON.stringify({ action }) });
const reply = (action) => ({ reply: JSON.stringify({ action }) });
const LOCKED = 'database-locked';
const UNLOCKED = 'database-unlocked';

// The shim in a context of its own, with a runtime that records every native
// message and leaves it pending until the test answers it.
function safari() {
    const sent = [];
    const pushes = [];
    const intervals = new Set();
    const warnings = [];

    const runtime = {
        sendNativeMessage: (name, envelope) => new Promise((resolve, reject) => {
            sent.push({ name, envelope, resolve, reject });
        }),
        connectNative: () => ({
            onMessage: { addListener: (fn) => pushes.push(fn) },
            onDisconnect: { addListener: () => {} },
            disconnect: () => {},
        }),
    };
    const context = vm.createContext({
        browser: { runtime },
        console: { log: () => {}, error: (...a) => warnings.push(a.join(' ')), warn: (...a) => warnings.push(a.join(' ')) },
        setInterval: (fn) => {
            intervals.add(fn);
            return fn;
        },
        clearInterval: (fn) => intervals.delete(fn),
    });
    vm.runInContext(SHIM, context);

    return {
        sent,
        warnings,
        connect() {
            const port = vm.runInContext("browser.runtime.connectNative('app.rowel.browser')", context);
            const heard = [];
            let disconnects = 0;
            port.onMessage.addListener((m) => heard.push(m.action));
            port.onDisconnect.addListener(() => disconnects++);
            return { port, heard, disconnects: () => disconnects };
        },
        // The heartbeat's two seconds pass.
        tick: () => [ ...intervals ].forEach((fn) => fn()),
        push: () => pushes.forEach((fn) => fn({ name: 'signal', userInfo: { seq: 99 } })),
        // Answer the oldest pending native message.
        async answer(response) {
            await settle();
            const next = sent.find((m) => !m.answered);
            assert.ok(next, 'a native message is outstanding');
            next.answered = true;
            next.resolve(response);
            await settle();
            // Out of the shim's realm, for deepEqual.
            return JSON.parse(JSON.stringify(next.envelope));
        },
        pending: () => sent.filter((m) => !m.answered).length,
    };
}

// A port whose key exchange has been answered: live, its heartbeat running.
async function connected() {
    const s = safari();
    const c = s.connect();
    c.port.postMessage({ action: 'change-public-keys' });
    await s.answer({ events: [ reply('change-public-keys') ], after: 0, connected: true });
    assert.deepEqual(c.heard, [ 'change-public-keys' ]);
    c.heard.length = 0;
    return { s, c };
}

test('requests go one at a time, handled in the order sent', async () => {
    const s = safari();
    const { port, heard } = s.connect();
    port.postMessage({ action: 'change-public-keys' });
    port.postMessage({ action: 'get-databasehash' });
    port.postMessage({ action: 'get-logins' });
    await settle();
    assert.equal(s.pending(), 1, 'one native message outstanding, not three');
    assert.deepEqual(JSON.parse(s.sent[0].envelope.request), { action: 'change-public-keys' });

    await s.answer({ events: [ reply('change-public-keys') ], after: 0, connected: true });
    assert.equal(s.sent.length, 2, 'the second goes once the first is answered');
    assert.deepEqual(JSON.parse(s.sent[1].envelope.request), { action: 'get-databasehash' });
    await s.answer({ events: [ reply('get-databasehash') ], after: 0, connected: true });
    await s.answer({ events: [ reply('get-logins') ], after: 0, connected: true });
    assert.deepEqual(heard, [ 'change-public-keys', 'get-databasehash', 'get-logins' ]);
});

test('a response delivered late still comes out before the ones sent after it', async () => {
    const s = safari();
    const { port, heard } = s.connect();
    port.postMessage({ action: 'change-public-keys' });
    port.postMessage({ action: 'get-databasehash' });
    await settle();
    // Safari might deliver the second message's answer first, if both were
    // out; with one outstanding it cannot even be sent until the first is in.
    const [ first ] = s.sent;
    assert.equal(s.sent.length, 1);
    await new Promise((resolve) => setTimeout(resolve, 20));
    first.answered = true;
    first.resolve({ events: [ reply('change-public-keys') ], after: 0, connected: true });
    await s.answer({ events: [ reply('get-databasehash') ], after: 0, connected: true });
    assert.deepEqual(heard, [ 'change-public-keys', 'get-databasehash' ]);
});

test('signals come out where the app sent them around the reply', async () => {
    const { s, c } = await connected();
    c.port.postMessage({ action: 'get-logins' });
    await s.answer({
        events: [ signal(1, LOCKED), signal(2, UNLOCKED), reply('get-logins'), signal(3, LOCKED) ],
        after: 2,
        connected: true,
    });
    assert.deepEqual(c.heard, [ LOCKED, UNLOCKED, 'get-logins', LOCKED ]);
    assert.deepEqual(s.warnings, []);
});

test('a signal heard already is dropped', async () => {
    const { s, c } = await connected();
    s.tick();
    await s.answer({ events: [ signal(1, LOCKED), signal(2, UNLOCKED) ], connected: true });
    s.tick();
    await s.answer({ events: [ signal(2, UNLOCKED), signal(3, LOCKED) ], connected: true });
    assert.deepEqual(c.heard, [ LOCKED, UNLOCKED, LOCKED ]);
});

test('events that disagree with `after` are emitted as listed, with a warning', async () => {
    const { s, c } = await connected();
    c.port.postMessage({ action: 'get-logins' });
    await s.answer({ events: [ reply('get-logins'), signal(4, LOCKED) ], after: 4, connected: true });
    assert.deepEqual(c.heard, [ 'get-logins', LOCKED ]);
    assert.equal(s.warnings.length, 1);
    assert.match(s.warnings[0], /signal 4/);
});

test('the heartbeat polls only while nothing is outstanding', async () => {
    const { s, c } = await connected();
    s.tick();
    assert.deepEqual(await s.answer({ events: [], connected: true }), { poll: true });

    // A request waiting on the user (a consent dialog): no polls pile up.
    c.port.postMessage({ action: 'associate' });
    await settle();
    s.tick();
    s.tick();
    s.tick();
    await settle();
    assert.equal(s.pending(), 1);
    assert.equal(s.sent.length, 3, 'no poll sent or queued behind the request');

    await s.answer({ events: [ reply('associate') ], after: 0, connected: true });
    s.tick();
    assert.deepEqual(await s.answer({ events: [ signal(1, LOCKED) ], connected: true }), { poll: true });
    assert.deepEqual(c.heard, [ 'associate', LOCKED ]);
});

test('a push is a cue to poll, once, and not a signal of its own', async () => {
    const { s, c } = await connected();
    s.push();
    s.push();
    await settle();
    assert.equal(s.pending(), 1, 'one poll for two pushes');
    assert.deepEqual(await s.answer({ events: [ signal(1, UNLOCKED) ], connected: true }), { poll: true });
    assert.deepEqual(c.heard, [ UNLOCKED ]);
});

for (const error of [ 'not-running', 'disconnected', 'bad-request' ]) {
    test(`${error} disconnects the port`, async () => {
        const s = safari();
        const c = s.connect();
        c.port.postMessage({ action: 'change-public-keys' });
        await s.answer({ events: [], connected: false, error, detail: 'test' });
        assert.equal(c.disconnects(), 1);
        assert.throws(() => c.port.postMessage({ action: 'get-logins' }), /disconnected port/);
    });
}

test('a heartbeat that finds the connection gone disconnects, after its signals', async () => {
    const { s, c } = await connected();
    s.tick();
    await s.answer({ events: [ signal(1, LOCKED) ], connected: false });
    assert.deepEqual(c.heard, [ LOCKED ]);
    assert.equal(c.disconnects(), 1);
    s.tick();
    await settle();
    assert.equal(s.pending(), 0, 'no heartbeat after the disconnect');
});

test('a rejected native message disconnects, and what was queued behind it is never sent', async () => {
    const s = safari();
    const c = s.connect();
    c.port.postMessage({ action: 'change-public-keys' });
    c.port.postMessage({ action: 'get-databasehash' });
    await settle();
    s.sent[0].answered = true;
    s.sent[0].reject(new Error('the app extension could not be launched'));
    await settle();
    assert.equal(c.disconnects(), 1);
    assert.equal(s.sent.length, 1);
});

test('after a disconnect a new port connects afresh', async () => {
    const { s, c } = await connected();
    c.port.postMessage({ action: 'get-logins' });
    await s.answer({ events: [], connected: false, error: 'disconnected', detail: 'the app went away' });
    assert.equal(c.disconnects(), 1);

    const again = s.connect();
    again.port.postMessage({ action: 'change-public-keys' });
    await s.answer({ events: [ signal(5, UNLOCKED), reply('change-public-keys') ], after: 5, connected: true });
    assert.deepEqual(again.heard, [ UNLOCKED, 'change-public-keys' ]);
    assert.equal(again.disconnects(), 0);
});

test('the port is installed through the polyfill\'s wrapped runtime', async () => {
    const sent = [];
    const chrome = {
        runtime: {
            id: 'rowel',
            lastError: null,
            getURL: () => 'safari-web-extension://rowel/',
            connectNative: () => ({
                onMessage: { addListener: () => {} },
                onDisconnect: { addListener: () => {} },
                disconnect: () => {},
                postMessage: () => {},
            }),
            sendNativeMessage: (name, envelope, callback) => {
                sent.push(envelope);
                setImmediate(() => callback({ events: [ reply('change-public-keys') ], after: 0, connected: true }));
            },
        },
    };
    const context = vm.createContext({ chrome, console: { log: () => {}, warn: () => {}, error: () => {} }, setInterval, clearInterval });
    context.globalThis = context;
    vm.runInContext(POLYFILL, context);
    vm.runInContext(SHIM, context);
    const port = vm.runInContext("browser.runtime.connectNative('app.rowel.browser')", context);
    assert.equal(port.constructor.name, 'SafariNativePort');
    const heard = [];
    port.onMessage.addListener((m) => heard.push(m.action));
    port.postMessage({ action: 'change-public-keys' });
    await settle();
    assert.deepEqual(heard, [ 'change-public-keys' ]);
    port.disconnect();
});
