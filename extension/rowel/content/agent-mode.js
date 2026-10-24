'use strict';

/**
 * Rowel: agent mode in the page (spec/05-delivery-modes.md §2).
 *
 * With agent mode on for this browser profile (an AI browser drives it), the
 * extension shows nothing in the page an agent could click, fills the one
 * entry the person admitted in Rowel's dialog, and checks that the secret it
 * typed does not end up showing in the page.
 *
 * Upstream files reach in through one-line gates: kpxcAgentMode.isOn() in
 * front of the icons, the autocomplete, the banners and the login popup;
 * kpxcAgentMode.fill() in place of kpxc.prepareCredentials; and the three
 * hooks afterFill (kpxcFill.fillInCredentials), noteTotp
 * (kpxcFill.setTOTPValue) and onPageLoad (initContentScript). The background
 * half is background/grants.js.
 *
 * The exposure check looks for the exact password and code in
 * document.body.innerText and in the value of every input that is not a
 * password field, and every textarea. Its limits: each frame's script scans
 * only its own document (cross-origin frames are opaque to it, and the
 * after-submit check runs in the top frame only); shadow roots are not
 * traversed, closed ones cannot be; text drawn as pixels (canvas, images) is
 * not seen; a page that echoes the secret later than the re-scan, without a
 * navigation, is missed. The values are kept in this closure only for the
 * checks and dropped once they have run.
 */
const kpxcAgentMode = {};
kpxcAgentMode.RESCAN_DELAY_MS = 1000;

// What one fill leaves for the exposure check: the secrets it typed, the one
// field the code went into (the one field holding the code that is no
// exposure), and the re-scan timer. One object per fill, replaced whole by
// `forget`, so nothing of a fill outlives it: the next fill's scan skips no
// field this one filled. What has been filled on this page is the page's,
// not a fill's, and lives apart (`filledOnPage`).
const freshFill = () => ({ password: '', totp: '', totpField: null, rescan: null });
let current = freshFill();
const filledOnPage = new WeakSet(); // Combinations (login and TOTP) already filled on this page

kpxcAgentMode.isOn = function() {
    return kpxc.settings?.agentMode === true;
};

// Only the fields the extension itself filled with a secret are left out of
// the scan: the password fields (which the selector skips anyway) and the one
// TOTP field the code went into. Every other login field is scanned — a page
// that copies the password into the username field is an exposure.
const filledFields = function() {
    return current.totpField ? [ current.totpField ] : [];
};

const forget = function() {
    clearTimeout(current.rescan);
    current = freshFill();
};

// The handoff for the next page load of this tab (background/grants.js, 60 s). No query string.
const handOff = function() {
    return sendMessage('agent_mode_set_filled', [ {
        password: current.password,
        totp: current.totp,
        url: trimURL(document.location.href)
    } ]);
};

const exposed = async function(fields) {
    kpxcAgentMode.clear(fields);
    forget();
    await sendMessage('agent_mode_take_filled'); // Drop the handoff as well
    await sendMessage('report_exposure', [ document.location.origin ]);
};

const scanOnce = async function() {
    const hit = kpxcAgentMode.scan({ password: current.password, totp: current.totp }, filledFields());
    if (hit.inText || hit.fields.length > 0) {
        await exposed(hit.fields);
        return true;
    }

    return false;
};

/**
 * In place of kpxc.prepareCredentials: fills the one admitted entry into the
 * latest login combination, as autoFillSingleEntry would, and its code when
 * the entry has one (get-totp carries the grant). Runs again as the observer
 * finds fields (the second step of a username-first sign-in) but fills each
 * once. Submits only through upstream's performAutoSubmit, i.e. with the
 * autoSubmit setting.
 */
kpxcAgentMode.fill = async function() {
    if (kpxc.credentials.length !== 1 || kpxc.combinations.length === 0) {
        logDebug('Agent mode: nothing to fill, expected one admitted entry and a login form.');
        return;
    }

    const entry = kpxc.credentials[0];
    const login = kpxc.combinations.findLast(c => c.username || c.password);
    const totp = kpxc.combinations.find(c => c.totp || c.totpInputs?.length > 0);
    const totpField = totp?.totp ?? totp?.totpInputs?.at(-1);

    await sendMessage('page_set_login_id', entry.uuid);

    // The code first, so an auto-submit after the password has it in place.
    // With autoFillSingleTotp on, fillInCredentials fills it itself.
    const totpByUpstream = login && kpxc.settings.autoFillSingleTotp;
    if (totpField && !totpByUpstream && kpxc.entryHasTotp(entry) && !filledOnPage.has(totp)) {
        filledOnPage.add(totp);
        await kpxcFill.fillTOTPFromUuid(totpField, entry.uuid);
    }

    if (login && !filledOnPage.has(login)) {
        filledOnPage.add(login);
        await kpxcFill.fillInCredentials(login, entry.login, entry.uuid);
    }
};

// From kpxcFill.fillInCredentials, once the values are in.
kpxcAgentMode.afterFill = async function(combination, entry) {
    if (!kpxcAgentMode.isOn() || !entry?.password) {
        return;
    }

    current.password = entry.password;
    await handOff();
    await kpxcAgentMode.check();
};

// From kpxcFill.setTOTPValue: the code, whether from get-totp or a KPH: {TOTP}
// field, and the field it goes into — the one field holding the code that is
// not an exposure. (A segmented code lands one digit per input, which the
// whole code never matches.)
kpxcAgentMode.noteTotp = async function(totp, field) {
    if (!kpxcAgentMode.isOn() || !totp) {
        return;
    }

    current.totpField = field ?? null;
    current.totp = String(totp);
    await handOff();
    await kpxcAgentMode.check();
};

/**
 * From initContentScript, before field detection, in the top frame. The
 * previous page of this tab (within a minute) may have been a fill:
 * clear-on-failure first (the same URL came back with a password field, so
 * every field holding the filled password is emptied before anything else
 * runs), then the after-submit exposure check.
 */
kpxcAgentMode.onPageLoad = async function() {
    if (!kpxcAgentMode.isOn() || window.self !== window.top) {
        return;
    }

    const record = await sendMessage('agent_mode_take_filled');
    if (!record?.password && !record?.totp) {
        return;
    }

    if (record.password
        && record.url === trimURL(document.location.href)
        && document.querySelector('input[type=password]')) {
        kpxcAgentMode.clear(kpxcAgentMode.fieldsWithValue(record.password));
    }

    current.password = record.password ?? '';
    current.totp = record.totp ?? '';
    await kpxcAgentMode.check();
};

// Scans now and once more after RESCAN_DELAY_MS (a page that echoes what was
// typed usually does so asynchronously), then drops the fill's state whole.
kpxcAgentMode.check = async function() {
    clearTimeout(current.rescan);
    if (await scanOnce()) {
        return;
    }

    current.rescan = setTimeout(async () => {
        await scanOnce();
        forget();
    }, kpxcAgentMode.RESCAN_DELAY_MS);
};

const nonEmpty = (s) => (typeof s === 'string' && s.length > 0 ? s : '');

// The code in the page text, but not inside a longer run of digits: a six-digit
// code is also found in phone numbers and order ids.
const codeInText = function(text, code) {
    const escaped = code.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    return new RegExp(`(^|\\D)${escaped}(\\D|$)`).test(text);
};

/**
 * Looks for the secrets in the page: in document.body.innerText, and in the
 * value of every input that is not a password field and every textarea,
 * except the fields in `ignore` (the login fields we fill ourselves). The
 * password matches as an exact substring everywhere; the code does so in
 * field values, but in the text only when no other digit touches it.
 * @param {{password?: string, totp?: string}} secrets   Empty ones are skipped
 * @param {Array<Element>} ignore   Fields whose value is not inspected
 * @param {Document} root           Defaults to this frame's document
 * @returns {{inText: boolean, fields: Array<Element>}}
 */
kpxcAgentMode.scan = function(secrets, ignore = [], root = document) {
    const found = { inText: false, fields: [] };
    const password = nonEmpty(secrets?.password);
    const totp = nonEmpty(secrets?.totp);
    const needles = [ password, totp ].filter(Boolean);
    if (needles.length === 0) {
        return found;
    }

    const text = root.body?.innerText ?? '';
    found.inText = (Boolean(password) && text.includes(password)) || (Boolean(totp) && codeInText(text, totp));

    for (const field of root.querySelectorAll('input:not([type=password]), textarea')) {
        if (!ignore.includes(field) && needles.some(n => field.value.includes(n))) {
            found.fields.push(field);
        }
    }

    return found;
};

// Every input or textarea whose value is exactly `value` (password fields included).
kpxcAgentMode.fieldsWithValue = function(value, root = document) {
    return Array.from(root.querySelectorAll('input, textarea')).filter(f => f.value === value);
};

// Empties the fields, with the events a framework listens for.
kpxcAgentMode.clear = function(fields) {
    for (const field of fields) {
        field.value = '';
        field.dispatchEvent(new Event('input', { bubbles: true }));
        field.dispatchEvent(new Event('change', { bubbles: true }));
    }
};
