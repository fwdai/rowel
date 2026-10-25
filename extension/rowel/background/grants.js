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
 * State lives in browser.storage.session, like upstream's submitted
 * credentials in credentials.js: in memory, gone when the browser quits.
 * The only hooks in upstream files are three one-liners in keepass.js
 * (retrieveCredentials, getTotp, updateDatabase); the message handlers for
 * the content script are registered at the bottom of this file.
 */
const grants = {};
grants.GRANT_TTL_MS = 15 * 60 * 1000; // A fill grant lives 15 minutes on the host
grants.FILLED_TTL_MS = 60 * 1000; // The next load must pick a handoff up within a minute
grants.PENDING_RETRY_CAP_MS = 10 * 1000;

const GRANTS_KEY = 'rowelGrants';
const FILLED_KEY = 'rowelFilled';
const REPORT_EXPOSURE_ACTION = 'report-exposure';

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

grants.rememberGrant = async function(origin, grantId) {
    if (!origin || !grantId) {
        return;
    }

    const map = await readMap(GRANTS_KEY);
    map[origin] = { grantId: grantId, expiresAt: grants.now() + grants.GRANT_TTL_MS };
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

grants.forgetGrant = async function(origin) {
    const map = await readMap(GRANTS_KEY);
    if (origin in map) {
        delete map[origin];
        await writeMap(GRANTS_KEY, map);
    }
};

// On database-locked (and any other host reset): nothing is served from a locked vault.
grants.clearAll = async function() {
    await grants.area().remove([ GRANTS_KEY, FILLED_KEY ]);
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
// Messages from content/agent-mode.js
//--------------------------------------------------------------------------

// The content script found a filled secret showing in the page. Tell the
// person, end the grant, and never reuse it, whatever the host answered.
grants.onExposure = async function(tab, args = []) {
    const [ origin ] = args;
    showNotification(tr('agentModeExposedNotification'));

    const grantId = await grants.grantFor(origin);
    if (grantId) {
        await grants.reportExposure(tab, [ grantId ]);
        await grants.forgetGrant(origin);
    }
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
}

// Exports for tests
if (typeof module === 'object') {
    module.exports = { grants };
}
