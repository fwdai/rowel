'use strict';

/**
 * Rowel: agent mode, background side (spec/05-delivery-modes.md §2,
 * spec/04-protocol.md "Extension-side actions").
 *
 * With agent mode on, every fill is a grant the person approves in Rowel. This
 * file keeps the grants the host hands back, sends them with get-logins and
 * get-totp, turns the grant refusals (codes 40–46) into notifications, reports
 * a secret the page showed, and holds the handoff a content script leaves for
 * the next page load of its tab (the after-submit exposure check and
 * clear-on-failure, content/agent-mode.js).
 *
 * It also takes the host's signals (spec/04-protocol.md "Signals to the
 * extension"): a grant an agent asked for is pushed as a clear `rowel-signal`
 * with no content, answered here with a sealed `get-signals`. A fill-grant
 * is remembered like one the person approved, turns agent mode on for its
 * lifetime when it was off, and wakes the tab at its origin; a grant-ended
 * signs the browser out of the sites it named and turns agent mode back off
 * once no agent grant is left. Agent mode the person turned on is never
 * touched.
 *
 * State lives in browser.storage.session, like upstream's submitted
 * credentials in credentials.js: in memory, gone when the browser quits.
 * The only hooks in upstream files are one-liners in keepass.js
 * (retrieveCredentials, getTotp, updateDatabase, reconnect) and the signal
 * route in client.js (onNativeMessage); the message handlers for the content
 * script are registered at the bottom of this file.
 */
const grants = {};
grants.GRANT_TTL_MS = 15 * 60 * 1000; // A fill grant lives 15 minutes on the host
grants.FILLED_TTL_MS = 60 * 1000; // The next load must pick a handoff up within a minute
grants.PENDING_RETRY_CAP_MS = 10 * 1000;

const GRANTS_KEY = 'rowelGrants';
const FILLED_KEY = 'rowelFilled';
const AGENT_GRANTS_KEY = 'rowelAgentGrants';
const AGENT_MODE_BY_GRANT_KEY = 'rowelAgentModeByGrant';
const REPORT_EXPOSURE_ACTION = 'report-exposure';
const GET_SIGNALS_ACTION = 'get-signals';

// Which refusals get a notification. The others show as the popup's error text only.
const GRANT_NOTIFICATIONS = {
    40: 'agentModeGrantDeniedNotification',
    41: 'agentModeGrantPendingNotification',
    45: 'agentModeRateLimitedNotification',
    46: 'agentModeAgentsDisabledNotification'
};

// Replaced by the unit test (tests/grants.spec.ts), which runs without a browser.
grants.area = () => browser.storage.session;
grants.now = () => Date.now();

grants.isOn = function() {
    return page?.settings?.agentMode === true;
};

grants.originOf = function(url) {
    try {
        return new URL(url).origin;
    } catch (_e) {
        return undefined;
    }
};

const readMap = async function(key) {
    const item = await grants.area().get(key);
    return item?.[key] ?? {};
};

const writeMap = function(key, map) {
    return grants.area().set({ [key]: map });
};

//--------------------------------------------------------------------------
// Grant map: origin → { grantId, expiresAt }
//--------------------------------------------------------------------------

// A grant from get-logins is given the host's window from now; a signalled one
// comes with the host's own expiry (expiresAt, absolute ms).
grants.rememberGrant = async function(origin, grantId, expiresAt = grants.now() + grants.GRANT_TTL_MS) {
    if (!origin || !grantId) {
        return;
    }

    const map = await readMap(GRANTS_KEY);
    map[origin] = { grantId: grantId, expiresAt: expiresAt };
    await writeMap(GRANTS_KEY, map);
};

// The live grant for an origin, or undefined. An expired one is dropped on the way.
grants.grantFor = async function(origin) {
    if (!origin) {
        return undefined;
    }

    const map = await readMap(GRANTS_KEY);
    const grant = map[origin];
    if (!grant) {
        return undefined;
    }

    if (grant.expiresAt <= grants.now()) {
        delete map[origin];
        await writeMap(GRANTS_KEY, map);
        return undefined;
    }

    return grant.grantId;
};

// A login was served to this profile under the grant. The host tells every
// extension of the lockbox about an agent's grant, and every one remembers
// it, but only the profile the agent's page is in ever fills under it —
// and only that profile is signed in, so only it is signed out at the end.
grants.markServed = async function(origin, grantId) {
    if (!origin || !grantId) {
        return;
    }

    const map = await readMap(GRANTS_KEY);
    if (map[origin]?.grantId === grantId) {
        map[origin].served = true;
        await writeMap(GRANTS_KEY, map);
    }
};

grants.forgetGrant = async function(origin) {
    const map = await readMap(GRANTS_KEY);
    if (origin in map) {
        delete map[origin];
        await writeMap(GRANTS_KEY, map);
    }
};

// On database-locked (and any other host reset): nothing is served from a locked
// vault, and every grant died with it — the host writes no row and sends no
// grant-ended for a lock, so the sign-out every grant's end owes happens here,
// for every origin a grant was held for. Agent mode turned on by one of them
// goes off too. A reset the host did not announce (the worker starting afresh
// with the port down) signs out as well: a grant the host can no longer be
// asked about is not one to keep a session open on.
grants.clearAll = async function() {
    const byGrant = await grants.agentModeByGrant();
    const held = await readMap(GRANTS_KEY);
    const signedIn = Object.keys(held).filter(origin => held[origin].served);
    await grants.area().remove([ GRANTS_KEY, FILLED_KEY, AGENT_GRANTS_KEY, AGENT_MODE_BY_GRANT_KEY ]);
    for (const origin of signedIn) {
        await grants.clearCookies(origin);
    }
    if (signedIn.length > 0) {
        showNotification(tr('agentModeGrantEndedNotification'));
    }
    if (byGrant) {
        await grants.setAgentMode(false);
    }
};

//--------------------------------------------------------------------------
// Agent mode by grant
//--------------------------------------------------------------------------

grants.agentModeByGrant = async function() {
    const item = await grants.area().get(AGENT_MODE_BY_GRANT_KEY);
    return item?.[AGENT_MODE_BY_GRANT_KEY] === true;
};

// The popup's path (popups/agent-mode.js → save_settings): the settings object
// is written whole and page.settings reloaded from it, so there is one way to
// change the flag and the popup, the options page and the content scripts all
// read the same value.
grants.setAgentMode = function(on) {
    return kpxcEvent.onSaveSettings(null, { ...page.settings, agentMode: on });
};

//--------------------------------------------------------------------------
// Requests
//--------------------------------------------------------------------------

// get-logins in agent mode. A live grant for the page's origin goes along as
// grantId (served without asking); otherwise agentMode: 'true' makes the host
// ask the person. A grantId in the reply is remembered for the origin.
// At most one retry: a grant the host no longer knows (GrantExpired) is
// forgotten and the page asked for afresh; a busy consent slot (GrantPending
// with retryAfterMs) is waited for once. Then give up and notify.
grants.sendGetLogins = async function(tab, messageData, nonce) {
    if (!grants.isOn()) {
        return keepassClient.sendMessage(kpActions.GET_LOGINS, tab, messageData, nonce);
    }

    const origin = grants.originOf(messageData.url);
    let grantId = await grants.grantFor(origin);
    let retried = false;

    for (;;) {
        const data = { ...messageData };
        if (grantId) {
            data.grantId = grantId;
        } else {
            data.agentMode = 'true';
        }

        const request = keepassClient.buildRequest(
            kpActions.GET_LOGINS,
            keepassClient.encrypt(data, nonce),
            nonce,
            keepass.clientID,
        );
        const response = await keepassClient.sendNativeMessage(request);
        const parsed = keepassClient.handleResponse(response, keepassClient.incrementedNonce(nonce), tab);
        if (parsed) {
            if (parsed.grantId) {
                await grants.rememberGrant(origin, parsed.grantId);
            }
            // A login came back under the grant: this profile is where the
            // sign-in happens, and so where the sign-out is owed.
            await grants.markServed(origin, grantId ?? parsed.grantId);
            return parsed;
        }

        const code = response?.errorCode;
        if (grantId && (code === kpErrors.GRANT_DENIED || code === kpErrors.GRANT_EXPIRED)) {
            await grants.forgetGrant(origin);
        }

        if (!retried) {
            retried = true;
            if (code === kpErrors.GRANT_EXPIRED && grantId) {
                grantId = undefined;
                nonce = keepassClient.getNonce();
                continue;
            }

            if (code === kpErrors.GRANT_PENDING && response?.retryAfterMs > 0) {
                await new Promise(resolve => setTimeout(resolve, Math.min(response.retryAfterMs, grants.PENDING_RETRY_CAP_MS)));
                nonce = keepassClient.getNonce();
                continue;
            }
        }

        grants.notify(code);
        return undefined;
    }
};

// The agent-mode fields for get-totp: agentMode always, so the host knows the
// code is for an agent and refuses it without a grant, plus the fill's grant
// when the requesting document's origin holds one. Keyed by the frame's own
// URL (sender.url, the content script's document), as get-logins keys the
// grant by the URL the content script sent (kpxc.url, that same document):
// a cross-origin login frame is its own origin, asks its own grant, and
// finds it here. The tab's URL stands in only for a sender without one.
grants.totpFields = async function(tab, sender) {
    if (!grants.isOn()) {
        return {};
    }

    const fields = { agentMode: 'true' };
    const grantId = await grants.grantFor(grants.originOf(sender?.url ?? tab?.url));
    if (grantId) {
        fields.grantId = grantId;
    }
    return fields;
};

// Fixed texts, nothing from the page or the entry.
grants.notify = function(code) {
    const key = GRANT_NOTIFICATIONS[code];
    if (key) {
        showNotification(tr(key));
    }
};

// report-exposure { grantId }: the host ends the grant and writes the audit row.
// The reply is the bare sealed success envelope. Also reachable as
// keepass.reportExposure (aliased below) beside the other actions.
grants.reportExposure = async function(tab, args = []) {
    const [ grantId ] = args;
    if (!grantId || !keepass.isConnected) {
        return false;
    }

    try {
        const taResponse = await keepass.testAssociation(tab, [ false ]);
        if (!taResponse) {
            return false;
        }

        const nonce = keepassClient.getNonce();
        const messageData = {
            action: REPORT_EXPOSURE_ACTION,
            grantId: grantId
        };

        const response = await keepassClient.sendMessage(REPORT_EXPOSURE_ACTION, tab, messageData, nonce);
        return Boolean(response);
    } catch (err) {
        logError(`reportExposure failed: ${err}`);
        return false;
    }
};

//--------------------------------------------------------------------------
// Signals from the host
//--------------------------------------------------------------------------

// Every entry from outside — a signal, a lock, an exposure — runs to its end
// before the next begins. They arrive from the native port and from content
// scripts with nothing ordering them, and each is a read-modify-write of the
// same session storage: a lock's cleanup overlapping a fill-grant from the
// unlock right after it (a lockbox switch sends the two back to back) would
// erase the new grant or turn off the agent mode it just turned on. One
// queue, nothing in it awaits anything outside grants.js that could call back
// in, and a failure is logged and lets the next entry run.
let queue = Promise.resolve();
const exclusive = function(work) {
    const run = queue.then(work);
    queue = run.catch(err => logError(`grants: ${err}`));
    return queue;
};

// The clear `rowel-signal` arrived (client.js), or the port just came back
// (keepass.reconnect): fetch what the host queued for this client and act on
// each. Nothing to fetch from a host we are not connected to; the signal
// waits in its queue for the reconnect.
grants.onSignal = function() {
    return exclusive(async () => {
        if (!keepass.isConnected) {
            return;
        }

        const taResponse = await keepass.testAssociation(null, [ false ]);
        if (!taResponse) {
            return;
        }

        const nonce = keepassClient.getNonce();
        const response = await keepassClient.sendMessage(GET_SIGNALS_ACTION, null, { action: GET_SIGNALS_ACTION }, nonce);
        for (const signal of response?.signals ?? []) {
            if (signal.signal === 'fill-grant') {
                await grants.onFillGrant(signal);
            } else if (signal.signal === 'grant-ended') {
                await grants.onGrantEnded(signal);
            }
        }
    });
};

// database-locked arrived (client.js): every grant died with the lock. Queued
// like a signal, so the sign-out and the reset finish before anything the
// unlock brings is acted on.
grants.onLocked = function() {
    return exclusive(() => grants.clearAll());
};

// fill-grant { grantId, origin, item, autoSubmit, expiresAtMs }: an agent's
// grant, approved in Rowel. Remembered like one the person approved from the
// page, so get-logins and get-totp at that origin carry it; marked agent-driven
// so grant-ended knows whether agent mode is still needed; and the tab at the
// origin is woken. Without a tab the grant waits: the content script asks on
// the next load at that origin, as agent mode already does.
grants.onFillGrant = async function({ grantId, origin, expiresAtMs }) {
    if (!origin || !grantId) {
        return;
    }

    await grants.rememberGrant(origin, grantId, expiresAtMs);
    const agentGrants = await readMap(AGENT_GRANTS_KEY);
    agentGrants[origin] = grantId;
    await writeMap(AGENT_GRANTS_KEY, agentGrants);

    // The host tells every profile; the agent drives one. Agent mode — which
    // hides the person's in-page UI — goes on only where the agent's page
    // is: now, in the profile with a tab at the origin, or later, when one
    // opens there (onTabUpdated). The others keep the grant in memory, so
    // they forget it with the rest when it ends, and change nothing else.
    if (await grants.hasTab(origin)) {
        await grants.enterAgentMode();
        await grants.wakeTab(origin);
    }
};

// Agent mode on for a grant's sake, remembered as such so the last grant-ended
// turns it back off; agent mode the person turned on is left as it is.
grants.enterAgentMode = async function() {
    if (!grants.isOn()) {
        await grants.area().set({ [AGENT_MODE_BY_GRANT_KEY]: true });
        await grants.setAgentMode(true);
    }
};

// A tab finished loading. If its origin holds an agent's live grant and this
// profile is not in agent mode, the agent's page has just arrived here: agent
// mode goes on and the tab retrieves under the grant. A profile already in
// agent mode fills on its own page load and needs nothing.
grants.onTabUpdated = function(tabId, changeInfo, tab) {
    if (changeInfo?.status !== 'complete' || !tab?.url || grants.isOn()) {
        return Promise.resolve();
    }

    return exclusive(async () => {
        const origin = grants.originOf(tab.url);
        const agentGrants = await readMap(AGENT_GRANTS_KEY);
        if (!agentGrants[origin] || await grants.grantFor(origin) !== agentGrants[origin]) {
            return;
        }

        await grants.enterAgentMode();
        await grants.wakeTab(origin);
    });
};

grants.hasTab = async function(origin) {
    return (await browser.tabs.query({})).some(tab => grants.originOf(tab.url) === origin);
};

// One tab at the origin, the active one first (page.retrieveCredentials serves
// only the active tab). The content script is told to redetect its fields:
// that reloads its settings, so a tab loaded before agent mode went on now
// fills as agent mode rather than through the person's icons, and ends in a
// retrieval of its own, which the cleared cache lets through to the host.
grants.wakeTab = async function(origin) {
    const matching = (await browser.tabs.query({})).filter(tab => grants.originOf(tab.url) === origin);
    const tab = matching.find(t => t.active) ?? matching[0];
    if (!tab) {
        return;
    }

    await page.clearLogins(tab.id);
    await browser.tabs.sendMessage(tab.id, { action: 'redetect_fields' }).catch(() => {});
};

// grant-ended { grantId, origins }: session end. The login the grant enabled is
// signed out of by deleting the sites' cookies, the grant is forgotten, and
// agent mode goes back off if this grant (or an earlier one) turned it on and
// no agent grant is left. An origin a newer grant holds is left whole: its
// session is that grant's to end, and ending it now would cut that grant
// off while the host still counts it active.
grants.onGrantEnded = async function({ grantId, origins = [] }) {
    const agentGrants = await readMap(AGENT_GRANTS_KEY);
    // The entries as stored, expiry not applied: the host's end of a grant
    // arrives at its expiry, and a look that dropped the expired entry on
    // the way would lose whether this profile was signed in under it.
    const held = await readMap(GRANTS_KEY);
    let signedOut = 0;
    for (const origin of origins) {
        const entry = held[origin];
        if (entry && entry.grantId !== grantId && entry.expiresAt > grants.now()) {
            continue;
        }
        // Signed out where it was signed in: only the profile a login was
        // served to under this grant holds a session to end.
        if (entry?.grantId === grantId && entry.served) {
            await grants.clearCookies(origin);
            signedOut += 1;
        }
        if (entry?.grantId === grantId) {
            await grants.forgetGrant(origin);
        }
        if (agentGrants[origin] === grantId) {
            delete agentGrants[origin];
        }
    }
    await writeMap(AGENT_GRANTS_KEY, agentGrants);

    if (signedOut > 0) {
        showNotification(tr('agentModeGrantEndedNotification'));
    }

    if (Object.keys(agentGrants).length === 0 && await grants.agentModeByGrant()) {
        await grants.area().remove(AGENT_MODE_BY_GRANT_KEY);
        await grants.setAgentMode(false);
    }
};

// Every cookie of the origin's registrable domain (spec/05 "Session end"), in
// every cookie store. The domain comes from page.getBaseDomainFromUrl, which
// probes the public suffix list the way upstream does for iframes. Safari
// gives an extension no cookies API: session end is best-effort there.
grants.clearCookies = async function(origin) {
    if (!grants.originOf(origin) || !browser.cookies) {
        return;
    }

    const domain = await page.getBaseDomainFromUrl(new URL(origin).hostname, origin);
    for (const cookie of await grants.cookiesOf(domain)) {
        const details = {
            url: (cookie.secure ? 'https://' : 'http://') + cookie.domain.replace(/^\./, '') + cookie.path,
            name: cookie.name,
            storeId: cookie.storeId
        };
        if (cookie.partitionKey) {
            details.partitionKey = cookie.partitionKey;
        }
        await browser.cookies.remove(details);
    }
};

// Every cookie of a registrable domain the extension can reach. One
// cookies.getAll() sees one store (the current one, unless storeId says
// which) and no partitioned cookie (unless partitionKey says which, {} being
// all of them), so a single call would leave a Firefox container, a private
// window or a partitioned cross-site frame signed in. Each store is asked in
// turn, for its unpartitioned cookies and, on a browser that knows the
// field, its partitioned ones too; a cookie keeps its store and partition
// so the removal names the same one.
grants.cookiesOf = async function(domain) {
    const stores = browser.cookies.getAllCookieStores ? await browser.cookies.getAllCookieStores() : [ undefined ];
    const found = new Map();
    for (const store of stores) {
        const details = { domain: domain };
        if (store?.id !== undefined) {
            details.storeId = store.id;
        }
        const batches = [ await browser.cookies.getAll(details) ];
        try {
            batches.push(await browser.cookies.getAll({ ...details, partitionKey: {} }));
        } catch (_e) {
            // The browser does not know partitioned cookies; there are none to find.
        }
        for (const cookie of batches.flat()) {
            const key = [ cookie.storeId, cookie.name, cookie.domain, cookie.path, JSON.stringify(cookie.partitionKey ?? null) ].join('\u0000');
            found.set(key, cookie);
        }
    }
    return [ ...found.values() ];
};

//--------------------------------------------------------------------------
// Messages from content/agent-mode.js
//--------------------------------------------------------------------------

// The content script found a filled secret showing in the page. Tell the
// person, end the grant, and never reuse it, whatever the host answered.
grants.onExposure = function(tab, args = []) {
    return exclusive(async () => {
        const [ origin ] = args;
        showNotification(tr('agentModeExposedNotification'));

        const grantId = await grants.grantFor(origin);
        if (grantId) {
            await grants.reportExposure(tab, [ grantId ]);
            await grants.forgetGrant(origin);
        }
    });
};

// Drops handoffs older than FILLED_TTL_MS. Returns true if any was dropped.
const pruneFilled = function(map) {
    let dropped = false;
    for (const [ tabId, record ] of Object.entries(map)) {
        if (!record || record.storedAt + grants.FILLED_TTL_MS <= grants.now()) {
            delete map[tabId];
            dropped = true;
        }
    }
    return dropped;
};

// Handoff from a fill to the next page load of the same tab: { password, totp, url }.
// One record per tab; the previous one is replaced.
grants.setFilled = async function(tab, args = []) {
    const [ record ] = args;
    if (!tab?.id || !record) {
        return;
    }

    const map = await readMap(FILLED_KEY);
    pruneFilled(map);
    map[String(tab.id)] = { ...record, storedAt: grants.now() };
    await writeMap(FILLED_KEY, map);
};

// Read-and-delete. Undefined when there is none or it is older than FILLED_TTL_MS.
grants.takeFilled = async function(tab) {
    if (!tab?.id) {
        return undefined;
    }

    const map = await readMap(FILLED_KEY);
    const key = String(tab.id);
    const record = map[key];
    const stale = pruneFilled(map);
    if (record || stale) {
        delete map[key];
        await writeMap(FILLED_KEY, map);
    }

    return record && record.storedAt + grants.FILLED_TTL_MS > grants.now() ? record : undefined;
};

// In the background (not in the unit test): the action beside the others on
// keepass, and the handlers for the content script. kpxcEvent.messageHandlers
// is built in event.js, which loads before this file.
if (typeof kpxcEvent === 'object') {
    keepass.reportExposure = grants.reportExposure;
    Object.assign(kpxcEvent.messageHandlers, {
        'agent_mode_set_filled': grants.setFilled,
        'agent_mode_take_filled': grants.takeFilled,
        'report_exposure': grants.onExposure
    });
    browser.tabs.onUpdated.addListener(grants.onTabUpdated);
}

// Exports for tests
if (typeof module === 'object') {
    module.exports = { grants };
}
