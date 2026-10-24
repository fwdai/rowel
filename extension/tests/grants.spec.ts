// Rowel: the grant map and the fill handoff in background/grants.js, as pure
// functions over a fake storage area, and the fields get-totp carries. The
// request side (sendGetLogins, reportExposure) needs the native host and is
// not covered here.
import { test, expect } from '@playwright/test';
import { grants } from '../rowel/background/grants.js';

// grants.isOn() reads the background's settings object, a global there.
declare global {
    // eslint-disable-next-line no-var
    var page: { settings?: { agentMode?: boolean } } | undefined;
}

// browser.storage.session with the shapes the WebExtension API returns
const fakeArea = () => {
    const store = {};
    return {
        store,
        async get(key) {
            return key in store ? { [key]: store[key] } : {};
        },
        async set(items) {
            Object.assign(store, items);
        },
        async remove(keys) {
            for (const key of [].concat(keys)) {
                delete store[key];
            }
        }
    };
};

let area;
let now;

test.beforeEach(() => {
    area = fakeArea();
    now = 1_700_000_000_000;
    grants.area = () => area;
    grants.now = () => now;
    globalThis.page = { settings: { agentMode: true } };
});

test.afterEach(() => {
    delete globalThis.page;
});

// get-totp in agent mode always says so, and carries the fill's grant when the
// tab's origin holds one — never the bare request the host would serve to a
// person's click.
test('totpFields says agentMode always and adds the grant the origin holds', async () => {
    expect(await grants.totpFields({ url: 'https://example.com/2fa' })).toEqual({ agentMode: 'true' });

    await grants.rememberGrant('https://example.com', 'g1');
    expect(await grants.totpFields({ url: 'https://example.com/2fa' })).toEqual({ agentMode: 'true', grantId: 'g1' });
    expect(await grants.totpFields({ url: 'https://other.example/2fa' })).toEqual({ agentMode: 'true' });
    expect(await grants.totpFields(undefined)).toEqual({ agentMode: 'true' });

    now += grants.GRANT_TTL_MS;
    expect(await grants.totpFields({ url: 'https://example.com/2fa' })).toEqual({ agentMode: 'true' });
});

// A cross-origin login frame is its own origin: its get-logins remembered a
// grant under the frame's URL, and its TOTP step finds that grant by the
// same key — the sender's URL — not the tab's. The tab's URL stands in only
// for a sender without one.
test('totpFields keys the grant by the requesting frame, not the tab', async () => {
    await grants.rememberGrant('https://login.example', 'g-frame');
    await grants.rememberGrant('https://top.example', 'g-top');
    const tab = { url: 'https://top.example/account' };

    expect(await grants.totpFields(tab, { url: 'https://login.example/frame?step=2' })).toEqual({ agentMode: 'true', grantId: 'g-frame' });
    expect(await grants.totpFields(tab, { url: 'https://top.example/account' })).toEqual({ agentMode: 'true', grantId: 'g-top' });
    expect(await grants.totpFields(tab, { url: 'https://other.example/' })).toEqual({ agentMode: 'true' });
    expect(await grants.totpFields(tab, {})).toEqual({ agentMode: 'true', grantId: 'g-top' });
    expect(await grants.totpFields(tab)).toEqual({ agentMode: 'true', grantId: 'g-top' });
});

test('totpFields adds nothing with agent mode off', async () => {
    globalThis.page = { settings: { agentMode: false } };
    await grants.rememberGrant('https://example.com', 'g1');
    expect(await grants.totpFields({ url: 'https://example.com/2fa' })).toEqual({});
});

test('a grant is remembered per origin and forgotten on request', async () => {
    await grants.rememberGrant('https://example.com', 'g1');
    await grants.rememberGrant('https://other.example', 'g2');

    expect(await grants.grantFor('https://example.com')).toBe('g1');
    expect(await grants.grantFor('https://other.example')).toBe('g2');
    expect(await grants.grantFor('https://unknown.example')).toBeUndefined();
    expect(await grants.grantFor(undefined)).toBeUndefined();

    await grants.forgetGrant('https://example.com');
    expect(await grants.grantFor('https://example.com')).toBeUndefined();
    expect(await grants.grantFor('https://other.example')).toBe('g2');
});

test('a newer grant for the same origin replaces the old one', async () => {
    await grants.rememberGrant('https://example.com', 'g1');
    await grants.rememberGrant('https://example.com', 'g2');
    expect(await grants.grantFor('https://example.com')).toBe('g2');
});

test('a grant expires 15 minutes after it was received', async () => {
    await grants.rememberGrant('https://example.com', 'g1');

    now += grants.GRANT_TTL_MS - 1;
    expect(await grants.grantFor('https://example.com')).toBe('g1');

    now += 1;
    expect(await grants.grantFor('https://example.com')).toBeUndefined();
    expect(area.store.rowelGrants).toEqual({});
});

test('nothing is remembered without an origin or a grant id', async () => {
    await grants.rememberGrant(undefined, 'g1');
    await grants.rememberGrant('https://example.com', undefined);
    expect(area.store.rowelGrants).toBeUndefined();
});

test('clearAll drops every grant and every handoff', async () => {
    await grants.rememberGrant('https://example.com', 'g1');
    await grants.setFilled({ id: 7 }, [ { password: 'p', totp: '', url: 'https://example.com/login' } ]);

    await grants.clearAll();
    expect(await grants.grantFor('https://example.com')).toBeUndefined();
    expect(await grants.takeFilled({ id: 7 })).toBeUndefined();
    expect(area.store).toEqual({});
});

test('originOf reduces a URL to its origin', () => {
    expect(grants.originOf('https://example.com/login?next=/home#x')).toBe('https://example.com');
    expect(grants.originOf('https://example.com:8443/')).toBe('https://example.com:8443');
    expect(grants.originOf('not a url')).toBeUndefined();
    expect(grants.originOf(undefined)).toBeUndefined();
});

test('the fill handoff is keyed by tab, read once, and gone after a minute', async () => {
    await grants.setFilled({ id: 7 }, [ { password: 'p', totp: '123456', url: 'https://example.com/login' } ]);
    await grants.setFilled({ id: 8 }, [ { password: 'q', totp: '', url: 'https://other.example/' } ]);

    expect(await grants.takeFilled({ id: 9 })).toBeUndefined();

    now += grants.FILLED_TTL_MS - 1;
    const record = await grants.takeFilled({ id: 7 });
    expect(record).toMatchObject({ password: 'p', totp: '123456', url: 'https://example.com/login' });
    expect(await grants.takeFilled({ id: 7 })).toBeUndefined();

    now += 1;
    expect(await grants.takeFilled({ id: 8 })).toBeUndefined();
    expect(area.store.rowelFilled).toEqual({});
});

test('a new handoff replaces the tab\'s previous one and prunes stale ones', async () => {
    await grants.setFilled({ id: 7 }, [ { password: 'old', totp: '', url: 'u' } ]);
    await grants.setFilled({ id: 8 }, [ { password: 'stale', totp: '', url: 'u' } ]);

    now += grants.FILLED_TTL_MS;
    await grants.setFilled({ id: 7 }, [ { password: 'new', totp: '', url: 'u' } ]);
    expect(Object.keys(area.store.rowelFilled)).toEqual([ '7' ]);
    expect((await grants.takeFilled({ id: 7 })).password).toBe('new');
});

test('setFilled ignores calls without a tab or a record', async () => {
    await grants.setFilled(undefined, [ { password: 'p' } ]);
    await grants.setFilled({ id: 7 }, []);
    expect(area.store.rowelFilled).toBeUndefined();
    expect(await grants.takeFilled(undefined)).toBeUndefined();
});
