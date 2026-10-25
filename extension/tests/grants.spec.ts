// Rowel: the grant map and the fill handoff in background/grants.js, as pure
// functions over a fake storage area, the fields get-totp carries, and the
// host's signals (fill-grant, grant-ended) against fake keepassClient, tabs
// and cookies globals. The request side (sendGetLogins, reportExposure) needs
// the native host and is not covered here.
import { test, expect } from '@playwright/test';
import { grants } from '../rowel/background/grants.js';

// grants.js reads the background's globals; the tests install fakes for them.
declare global {
    /* eslint-disable no-var */
    var page: { settings?: { agentMode?: boolean }; getBaseDomainFromUrl?; clearLogins? } | undefined;
    var keepass: { isConnected: boolean; testAssociation } | undefined;
    var keepassClient: { getNonce; sendMessage } | undefined;
    var kpxcEvent: { onSaveSettings } | undefined;
    var browser: { tabs; cookies; storage } | undefined;
    var showNotification: ((text: string) => void) | undefined;
    var tr: ((key: string) => string) | undefined;
    var logError: ((text: string) => void) | undefined;
    /* eslint-enable no-var */
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

// What the fakes record and what they answer
let signals: object[];
let sent: { action: string; messageData: object }[];
let tabList: { id: number; url: string; active: boolean }[];
let tabMessages: { tabId: number; message: object }[];
let clearedTabs: number[];
let cookieJar: object[];
let cookieStores: string[];
let removedCookies: object[];
let savedAgentMode: boolean[];
let notifications: string[];

test.beforeEach(() => {
    area = fakeArea();
    now = 1_700_000_000_000;
    grants.area = () => area;
    grants.now = () => now;

    signals = [];
    sent = [];
    tabList = [];
    tabMessages = [];
    clearedTabs = [];
    cookieJar = [];
    cookieStores = [ '0', '1' ];
    removedCookies = [];
    savedAgentMode = [];
    notifications = [];

    globalThis.page = {
        settings: { agentMode: true },
        // The registrable domain, as upstream's public-suffix probe would find it
        async getBaseDomainFromUrl(hostname) {
            return hostname.split('.').slice(-2).join('.');
        },
        async clearLogins(tabId) {
            clearedTabs.push(tabId);
        }
    };
    globalThis.keepass = {
        isConnected: true,
        async testAssociation() {
            return true;
        }
    };
    globalThis.keepassClient = {
        getNonce: () => 'nonce',
        async sendMessage(action, _tab, messageData) {
            sent.push({ action, messageData });
            return { signals };
        }
    };
    globalThis.kpxcEvent = {
        // save_settings: the settings object is written whole and reloaded
        async onSaveSettings(_tab, settings) {
            savedAgentMode.push(settings.agentMode);
            globalThis.page.settings = settings;
        }
    };
    globalThis.browser = {
        tabs: {
            async query() {
                return tabList;
            },
            async sendMessage(tabId, message) {
                tabMessages.push({ tabId, message });
            }
        },
        // The real API's scoping: one store per call (the first when none is
        // named), unpartitioned cookies unless partitionKey is given, {} being
        // all of them. A fake that answered everything would hide exactly the
        // cookies a one-call clearing misses.
        cookies: {
            async getAllCookieStores() {
                return cookieStores.map(id => ({ id }));
            },
            async getAll({ domain, storeId, partitionKey }) {
                const store = storeId ?? cookieStores[0];
                return cookieJar.filter(c => c.domain.replace(/^\./, '').endsWith(domain)
                    && c.storeId === store
                    && (partitionKey ? true : !c.partitionKey));
            },
            async remove(details) {
                removedCookies.push(details);
            }
        },
        storage: { session: area }
    };
    globalThis.showNotification = text => notifications.push(text);
    globalThis.tr = key => key;
    globalThis.logError = text => {
        throw new Error(text);
    };
});

test.afterEach(() => {
    for (const name of [ 'page', 'keepass', 'keepassClient', 'kpxcEvent', 'browser', 'showNotification', 'tr', 'logError' ]) {
        delete globalThis[name];
    }
});

const fillGrant = (grantId, origin, expiresAtMs) => ({
    signal: 'fill-grant',
    grantId,
    origin,
    item: { id: 'i1', title: 'Example', username: 'me' },
    autoSubmit: false,
    expiresAtMs
});

const grantEnded = (grantId, origins) => ({ signal: 'grant-ended', grantId, origins });

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
    globalThis.page.settings = { agentMode: false };
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

test('a grant given an expiry of its own keeps it', async () => {
    await grants.rememberGrant('https://example.com', 'g1', now + 1000);

    now += 999;
    expect(await grants.grantFor('https://example.com')).toBe('g1');

    now += 1;
    expect(await grants.grantFor('https://example.com')).toBeUndefined();
});

test('nothing is remembered without an origin or a grant id', async () => {
    await grants.rememberGrant(undefined, 'g1');
    await grants.rememberGrant('https://example.com', undefined);
    expect(area.store.rowelGrants).toBeUndefined();
});

test('clearAll drops every grant and every handoff', async () => {
    await grants.rememberGrant('https://example.com', 'g1');
    await grants.setFilled({ id: 7 }, [ { password: 'p', totp: '', url: 'https://example.com/login' } ]);

    await grants.onLocked();
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

//--------------------------------------------------------------------------
// Signals
//--------------------------------------------------------------------------

// The wake-up is answered with one sealed get-signals; the fill-grant it
// carries is remembered for its origin with the host's expiry, marked
// agent-driven, and exactly one tab at that origin is woken — the active one.
test('a fill-grant remembers the grant with the host\'s expiry and wakes the tab at its origin', async () => {
    tabList = [
        { id: 1, url: 'https://other.example/', active: false },
        { id: 2, url: 'https://github.com/login', active: false },
        { id: 3, url: 'https://github.com/settings', active: true },
        { id: 4, url: 'https://evil.github.com.example/', active: true }
    ];
    signals = [ fillGrant('g1', 'https://github.com', now + 5000) ];

    await grants.onSignal();

    expect(sent).toEqual([ { action: 'get-signals', messageData: { action: 'get-signals' } } ]);
    expect(area.store.rowelGrants).toEqual({ 'https://github.com': { grantId: 'g1', expiresAt: now + 5000 } });
    expect(area.store.rowelAgentGrants).toEqual({ 'https://github.com': 'g1' });
    expect(clearedTabs).toEqual([ 3 ]);
    expect(tabMessages).toEqual([ { tabId: 3, message: { action: 'redetect_fields' } } ]);

    now += 5000;
    expect(await grants.grantFor('https://github.com')).toBeUndefined();
});

test('a fill-grant without a tab at its origin only remembers the grant', async () => {
    tabList = [ { id: 1, url: 'https://other.example/', active: true } ];
    signals = [ fillGrant('g1', 'https://github.com', now + 5000) ];

    await grants.onSignal();

    expect(await grants.grantFor('https://github.com')).toBe('g1');
    expect(clearedTabs).toEqual([]);
    expect(tabMessages).toEqual([]);
});

test('with no active tab at the origin the first one is woken', async () => {
    tabList = [
        { id: 2, url: 'https://github.com/login', active: false },
        { id: 3, url: 'https://github.com/settings', active: false }
    ];
    signals = [ fillGrant('g1', 'https://github.com', now + 5000) ];

    await grants.onSignal();
    expect(tabMessages.map(m => m.tabId)).toEqual([ 2 ]);
});

// Agent mode the person turned on is never touched: a grant arriving and
// ending while it is on leaves it on and leaves no trace of having tried.
test('agent mode the person turned on stays on through a grant\'s life', async () => {
    signals = [ fillGrant('g1', 'https://github.com', now + 5000) ];
    await grants.onSignal();
    expect(savedAgentMode).toEqual([]);
    expect(area.store.rowelAgentModeByGrant).toBeUndefined();

    signals = [ grantEnded('g1', [ 'https://github.com' ]) ];
    await grants.onSignal();
    expect(savedAgentMode).toEqual([]);
    expect(globalThis.page.settings.agentMode).toBe(true);
});

// Off before: the first agent grant turns it on, the last grant-ended turns
// it back off. A second grant in between does not turn it on twice, and the
// first grant ending does not turn it off while the second is live.
test('agent mode flips on with the first agent grant and off with the last grant-ended', async () => {
    globalThis.page.settings = { agentMode: false };

    signals = [ fillGrant('g1', 'https://github.com', now + 5000) ];
    await grants.onSignal();
    expect(savedAgentMode).toEqual([ true ]);
    expect(globalThis.page.settings.agentMode).toBe(true);
    expect(area.store.rowelAgentModeByGrant).toBe(true);

    signals = [ fillGrant('g2', 'https://gitlab.com', now + 5000) ];
    await grants.onSignal();
    expect(savedAgentMode).toEqual([ true ]);

    signals = [ grantEnded('g1', [ 'https://github.com' ]) ];
    await grants.onSignal();
    expect(savedAgentMode).toEqual([ true ]);
    expect(area.store.rowelAgentGrants).toEqual({ 'https://gitlab.com': 'g2' });

    signals = [ grantEnded('g2', [ 'https://gitlab.com' ]) ];
    await grants.onSignal();
    expect(savedAgentMode).toEqual([ true, false ]);
    expect(globalThis.page.settings.agentMode).toBe(false);
    expect(area.store.rowelAgentModeByGrant).toBeUndefined();
    expect(area.store.rowelAgentGrants).toEqual({});
});

// Session end: exactly the cookies getAll({ domain }) returned for the
// origin's registrable domain are removed, by the URL rebuilt from each
// cookie's own domain and path, in each cookie's own store.
test('a grant-ended deletes the domain\'s cookies, forgets the grant and notifies', async () => {
    await grants.rememberGrant('https://github.com', 'g1');
    await grants.rememberGrant('https://other.example', 'g-other');
    cookieJar = [
        { name: 'session', domain: '.github.com', path: '/', secure: true, storeId: '0' },
        { name: 'pref', domain: 'gist.github.com', path: '/gists', secure: false, storeId: '1' },
        { name: 'unrelated', domain: 'other.example', path: '/', secure: true, storeId: '0' }
    ];
    signals = [ grantEnded('g1', [ 'https://github.com' ]) ];

    await grants.onSignal();

    expect(removedCookies).toEqual([
        { url: 'https://github.com/', name: 'session', storeId: '0' },
        { url: 'http://gist.github.com/gists', name: 'pref', storeId: '1' }
    ]);
    expect(await grants.grantFor('https://github.com')).toBeUndefined();
    expect(await grants.grantFor('https://other.example')).toBe('g-other');
    expect(notifications).toEqual([ 'agentModeGrantEndedNotification' ]);
});

// A session lives in whichever store the agent's browser profile used — a
// Firefox container, a private window — and may be a partitioned cookie in a
// cross-site frame. Each store is asked in turn, partitioned cookies are
// asked for by name, and each removal names the cookie's own store and
// partition; a store the browser does not list is not reachable and is left.
test('session end reaches every cookie store and partitioned cookies', async () => {
    await grants.rememberGrant('https://github.com', 'g1');
    cookieStores = [ '0', 'firefox-container-2' ];
    cookieJar = [
        { name: 'session', domain: '.github.com', path: '/', secure: true, storeId: '0' },
        { name: 'session', domain: '.github.com', path: '/', secure: true, storeId: 'firefox-container-2' },
        { name: 'embedded', domain: 'github.com', path: '/', secure: true, storeId: 'firefox-container-2', partitionKey: { topLevelSite: 'https://example.com' } },
        { name: 'session', domain: '.github.com', path: '/', secure: true, storeId: 'unlisted' }
    ];
    signals = [ grantEnded('g1', [ 'https://github.com' ]) ];

    await grants.onSignal();

    expect(removedCookies).toEqual([
        { url: 'https://github.com/', name: 'session', storeId: '0' },
        { url: 'https://github.com/', name: 'session', storeId: 'firefox-container-2' },
        { url: 'https://github.com/', name: 'embedded', storeId: 'firefox-container-2', partitionKey: { topLevelSite: 'https://example.com' } }
    ]);
});

// A browser that lists no stores and knows no partitionKey (the call throws
// on the unknown field) still has its one store cleared.
test('session end on a browser without store listing or partitioned cookies', async () => {
    await grants.rememberGrant('https://github.com', 'g1');
    cookieJar = [ { name: 'session', domain: '.github.com', path: '/', secure: true, storeId: '0' } ];
    const plain = globalThis.browser.cookies.getAll;
    delete globalThis.browser.cookies.getAllCookieStores;
    globalThis.browser.cookies.getAll = async (details) => {
        if ('partitionKey' in details) {
            throw new Error('Unexpected property: partitionKey');
        }
        return plain(details);
    };
    signals = [ grantEnded('g1', [ 'https://github.com' ]) ];

    await grants.onSignal();

    expect(removedCookies).toEqual([ { url: 'https://github.com/', name: 'session', storeId: '0' } ]);
});

// An origin a newer grant holds is that grant's: ending the older one must
// not sign the browser out from under the one the host still counts active.
test('a grant-ended for an older grant leaves a newer grant at the same origin, session included', async () => {
    globalThis.page.settings = { agentMode: false };
    cookieJar = [ { name: 'session', domain: '.github.com', path: '/', secure: true, storeId: '0' } ];
    signals = [ fillGrant('g1', 'https://github.com', now + 5000), fillGrant('g2', 'https://github.com', now + 9000) ];
    await grants.onSignal();

    signals = [ grantEnded('g1', [ 'https://github.com' ]) ];
    await grants.onSignal();

    expect(removedCookies).toEqual([]);
    expect(notifications).toEqual([]);
    expect(await grants.grantFor('https://github.com')).toBe('g2');
    expect(area.store.rowelAgentGrants).toEqual({ 'https://github.com': 'g2' });
    expect(globalThis.page.settings.agentMode).toBe(true);

    signals = [ grantEnded('g2', [ 'https://github.com' ]) ];
    await grants.onSignal();
    expect(removedCookies).toEqual([ { url: 'https://github.com/', name: 'session', storeId: '0' } ]);
    expect(await grants.grantFor('https://github.com')).toBeUndefined();
    expect(globalThis.page.settings.agentMode).toBe(false);
});

test('onSignal does nothing with an empty signals list', async () => {
    await grants.onSignal();
    expect(sent).toHaveLength(1);
    expect(area.store).toEqual({});
    expect(tabMessages).toEqual([]);
    expect(savedAgentMode).toEqual([]);
    expect(notifications).toEqual([]);
});

test('onSignal asks nothing of a host it is not connected to', async () => {
    globalThis.keepass.isConnected = false;
    signals = [ fillGrant('g1', 'https://github.com', now + 5000) ];

    await grants.onSignal();
    expect(sent).toEqual([]);
    expect(area.store).toEqual({});
});

test('onSignal asks nothing when the association test fails', async () => {
    globalThis.keepass.testAssociation = async () => false;
    await grants.onSignal();
    expect(sent).toEqual([]);
});

// A lock ends every grant on the host with no grant-ended to follow, so the
// sign-out each grant's end owes happens here, for every origin held — the
// person's own agent-mode grants as much as an agent's.
test('clearAll signs the browser out of every origin a grant was held for', async () => {
    await grants.rememberGrant('https://github.com', 'g1');
    signals = [ fillGrant('g2', 'https://bank.example', now + 5000) ];
    await grants.onSignal();
    cookieJar = [
        { name: 'session', domain: '.github.com', path: '/', secure: true, storeId: '0' },
        { name: 'sid', domain: 'bank.example', path: '/', secure: true, storeId: '0' },
        { name: 'unrelated', domain: 'other.example', path: '/', secure: true, storeId: '0' }
    ];

    await grants.onLocked();

    expect(removedCookies.map(c => c.name).sort()).toEqual([ 'session', 'sid' ]);
    expect(notifications).toEqual([ 'agentModeGrantEndedNotification' ]);
    expect(area.store).toEqual({});
});

// A lockbox switch sends database-locked and database-unlocked back to back,
// and a fill-grant may follow at once: the lock's cleanup runs to its end
// first, so the new grant and the agent mode it turns on survive it.
test('a lock and a fill-grant arriving together are taken in order', async () => {
    globalThis.page.settings = { agentMode: false };
    await grants.rememberGrant('https://github.com', 'g-old');
    cookieJar = [ { name: 'session', domain: '.github.com', path: '/', secure: true, storeId: '0' } ];
    signals = [ fillGrant('g-new', 'https://github.com', now + 5000) ];

    const locked = grants.onLocked();
    const signalled = grants.onSignal();
    await Promise.all([ locked, signalled ]);

    expect(removedCookies.map(c => c.name)).toEqual([ 'session' ]);
    expect(await grants.grantFor('https://github.com')).toBe('g-new');
    expect(area.store.rowelAgentGrants).toEqual({ 'https://github.com': 'g-new' });
    expect(area.store.rowelAgentModeByGrant).toBe(true);
    expect(globalThis.page.settings.agentMode).toBe(true);
});

// A failure in one entry is logged and does not hold the queue.
test('a failing entry does not block the next', async () => {
    globalThis.logError = text => notifications.push(`logged: ${text}`);
    globalThis.keepass.testAssociation = async () => {
        throw new Error('port gone');
    };
    await grants.onSignal();
    expect(notifications).toEqual([ 'logged: grants: Error: port gone' ]);

    globalThis.keepass.testAssociation = async () => true;
    signals = [ fillGrant('g1', 'https://github.com', now + 5000) ];
    await grants.onSignal();
    expect(await grants.grantFor('https://github.com')).toBe('g1');
});

test('clearAll with nothing held signs out of nothing', async () => {
    cookieJar = [ { name: 'session', domain: '.github.com', path: '/', secure: true, storeId: '0' } ];
    await grants.onLocked();
    expect(removedCookies).toEqual([]);
    expect(notifications).toEqual([]);
});

// A lock ends every grant on the host, agent grants included, so the agent
// mode one of them turned on goes off with them.
test('clearAll clears the agent grants and turns off the agent mode a grant turned on', async () => {
    globalThis.page.settings = { agentMode: false };
    signals = [ fillGrant('g1', 'https://github.com', now + 5000) ];
    await grants.onSignal();
    expect(area.store.rowelAgentGrants).toEqual({ 'https://github.com': 'g1' });
    expect(area.store.rowelAgentModeByGrant).toBe(true);

    await grants.onLocked();
    expect(area.store).toEqual({});
    expect(savedAgentMode).toEqual([ true, false ]);
});

test('clearAll leaves agent mode the person turned on alone', async () => {
    signals = [ fillGrant('g1', 'https://github.com', now + 5000) ];
    await grants.onSignal();

    await grants.onLocked();
    expect(area.store).toEqual({});
    expect(savedAgentMode).toEqual([]);
    expect(globalThis.page.settings.agentMode).toBe(true);
});
