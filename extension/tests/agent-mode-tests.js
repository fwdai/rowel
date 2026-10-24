'use strict';

// Rowel: the agent-mode checks run inside agent-mode.html, in the style of
// tests.js. Each check writes a result row; tests/agent-mode.spec.ts reads
// the cards. The background and the native host are not here: the stub in
// browser-stub.js answers the messages, so the grant request itself is not
// exercised (that is background/grants.js, unit-tested in grants.spec.ts).

const Tests = {
    INPUT_FIELDS: '#input-field-results'
};

const AgentCards = {
    ON: '#agent-on-results',
    RETRIEVE: '#agent-retrieve-results',
    SCAN: '#agent-scan-results',
    EXPOSURE: '#agent-exposure-results',
    FAILURE: '#agent-failure-results',
    OFF: '#agent-off-results'
};

const ENTRY = {
    uuid: 'agent-entry-1',
    name: 'Example',
    login: 'alice',
    password: 'S3cret!pass-7Q',
    totp: 'otpauth://totp/Example:alice?secret=JBSWY3DPEHPK3PXP',
    group: '',
    stringFields: []
};

const SECOND_ENTRY = {
    uuid: 'agent-entry-2',
    name: 'Example two',
    login: 'bob',
    password: 'Other!pass-9K',
    group: '',
    stringFields: []
};

const CODE = '246810';
const FAILED_PASSWORD = 'Echoed!back-3Z';

function createResult(card, res, text) {
    const icon = kpxcUI.createElement('i', res ? 'fa fa-check' : 'fa fa-close', { id: text });
    const span = kpxcUI.createElement('span', '', '', text);
    const br = document.createElement('br');

    document.querySelector(card).appendMultiple(icon, span, br);
}

const settingsWith = (agentMode) => ({
    agentMode: agentMode,
    autoCompleteUsernames: true,
    autoFillSingleEntry: false,
    autoFillSingleTotp: false,
    autoSubmit: false,
    colorTheme: 'light',
    credentialSorting: 'sortByGroupAndTitle',
    redirectAllowance: 3,
    saveDomainOnly: true,
    showGroupNameInAutocomplete: true,
    showLoginFormIcon: true,
    showLoginNotifications: true,
    showNotifications: false,
    showOTPIcon: true,
    sitePreferences: [],
    useCompactMode: false,
    usePasswordGeneratorIcons: true
});

const DEFAULT_REPLIES = {
    banner_get_position: 1,
    get_connected_database: { identifier: 'test' },
    get_totp: CODE,
    page_get_autosubmit_performed: false,
    page_get_manual_fill: 0,
    password_get_filled: false
};

const iconCount = () => kpxcIcons.icons.length + kpxcUsernameIcons.icons.length
    + kpxcPasswordIcons.icons.length + kpxcTOTPIcons.icons.length;

// The wrappers upstream appends to body for its icons and banners (a closed shadow
// root each): whatever was not there when the page loaded.
const pageChildren = new Set(document.body.children);
const injectedWrappers = () => Array.from(document.body.children).filter(e => !pageChildren.has(e));

const show = (id) => (document.getElementById(id).style.display = 'block');
const hide = (id) => (document.getElementById(id).style.display = 'none');
const wait = (ms) => new Promise(resolve => setTimeout(resolve, ms));

// A fresh page as far as the content scripts are concerned
function reset(agentMode) {
    kpxc.settings = settingsWith(agentMode);
    kpxc.credentials = [];
    kpxc.combinations = [];
    kpxc.inputs = [];
    kpxc.url = null;
    kpxc.submitUrl = null;
    kpxc.databaseState = DatabaseState.DISCONNECTED;
    kpxcForm.savedForms = [];
    kpxcIcons.icons = [];
    kpxcUsernameIcons.icons = [];
    kpxcPasswordIcons.icons = [];
    kpxcTOTPIcons.icons = [];
    kpxcUserAutocomplete.clear();
    kpxcUserAutocomplete.autocompleteList = [];
    kpxcTOTPAutocomplete.clear();
    kpxcTOTPAutocomplete.autocompleteList = [];

    for (const form of document.forms) {
        form.reset();
    }
    document.getElementById('agent-echo-text').textContent = '';
    document.getElementById('agent-echo-input').value = '';
    document.getElementById('agent-echo-area').value = '';

    for (const wrapper of injectedWrappers()) {
        wrapper.remove();
    }

    browserStub.reset({ ...DEFAULT_REPLIES });
}

// Agent mode on: fields are detected, nothing is injected, the one entry is filled.
async function testAgentModeOn() {
    const card = AgentCards.ON;
    reset(true);
    show('agent-login');

    kpxcAssert(kpxcAgentMode.isOn(), true, card, 'isOn() follows the agentMode setting');

    await kpxc.initCredentialFields();
    kpxcAssert(kpxc.combinations.length, 2, card, 'The login form and the TOTP field are still detected');
    kpxcAssert(iconCount(), 0, card, 'No username, password or TOTP icons are created');
    kpxcAssert(injectedWrappers().length, 0, card, 'Nothing is injected into the page');

    kpxc.credentials = [ ENTRY ];
    await kpxc.prepareCredentials();
    kpxcAssert($('#agent-username').value, ENTRY.login, card, 'The admitted username is filled');
    kpxcAssert($('#agent-password').value, ENTRY.password, card, 'The admitted password is filled');
    kpxcAssert($('#agent-otp').value, CODE, card, 'The TOTP is filled with the code from get_totp');
    kpxcAssert(kpxcUserAutocomplete.autocompleteList.length, 0, card, 'No autocomplete is attached');
    kpxcAssert(browserStub.sentFor('popup_login').length, 0, card, 'No login popup is offered');
    kpxcAssert(browserStub.sentFor('agent_mode_set_filled').length > 0, true, card, 'The fill is handed to the next page load');
    kpxcAssert(browserStub.sentFor('agent_mode_set_filled').at(-1).args[0].url, trimURL(document.location.href), card, 'The handoff carries the page URL without a query string');
    kpxcAssert(browserStub.sentFor('report_exposure').length, 0, card, 'A page that does not show the secret reports no exposure');

    // Filling again (the observer found the same fields once more) is a no-op
    const fills = browserStub.sentFor('get_totp').length;
    await kpxc.prepareCredentials();
    kpxcAssert(browserStub.sentFor('get_totp').length, fills, card, 'The same fields are not filled twice');

    await kpxcBanner.create({ username: 'alice', password: 'x', url: '', usernameExists: false, list: [] });
    kpxcAssert(kpxcBanner.created, false, card, 'No save banner is created');

    await kpxcCustomLoginFieldsBanner.create();
    kpxcAssert(kpxcCustomLoginFieldsBanner.created, false, card, 'No custom fields banner is created');

    await assertInputFields('basic1', 2);
    hide('agent-login');
}

// Agent mode retrieves on its own: with autoRetrieveCredentials off the form is still filled.
// Runs the real path: initCredentialFields → kpxc.retrieveCredentials → retrieve_credentials
// (the stub answers with the admitted entry) → prepareCredentials → kpxcAgentMode.fill.
async function testRetrievesWithoutAutoRetrieve() {
    const card = AgentCards.RETRIEVE;
    reset(true);
    kpxc.settings.autoRetrieveCredentials = false;
    kpxc.databaseState = DatabaseState.UNLOCKED;
    browserStub.replies.retrieve_credentials = [ ENTRY ];
    show('agent-login');

    await kpxc.initCredentialFields();
    kpxcAssert(browserStub.sentFor('retrieve_credentials').length, 1, card, 'Agent mode asks for credentials although autoRetrieveCredentials is off');
    kpxcAssert(browserStub.sentFor('retrieve_credentials')[0].args[0], document.location.href, card, 'The request names the page URL');
    kpxcAssert($('#agent-username').value, ENTRY.login, card, 'The username is filled');
    kpxcAssert($('#agent-password').value, ENTRY.password, card, 'The password is filled');
    kpxcAssert($('#agent-otp').value, CODE, card, 'The code is filled');
    kpxcAssert(injectedWrappers().length, 0, card, 'Still nothing is injected into the page');
    hide('agent-login');
    await wait(kpxcAgentMode.RESCAN_DELAY_MS * 2);

    // Agent mode off with the setting off: upstream waits for the user, as before
    reset(false);
    kpxc.settings.autoRetrieveCredentials = false;
    kpxc.databaseState = DatabaseState.UNLOCKED;
    browserStub.replies.retrieve_credentials = [ ENTRY ];
    show('agent-login');

    await kpxc.initCredentialFields();
    kpxcAssert(browserStub.sentFor('retrieve_credentials').length, 0, card, 'With agent mode off, autoRetrieveCredentials off still means no request');
    kpxcAssert($('#agent-password').value, '', card, 'With agent mode off nothing is filled');
    hide('agent-login');
}

// The pure scan and clear functions
async function testScanAndClear() {
    const card = AgentCards.SCAN;
    reset(true);
    show('agent-echo');

    const other = $('#agent-echo-username');
    other.value = 'alice';
    $('#agent-echo-password').value = FAILED_PASSWORD;
    $('#agent-echo-text').textContent = 'Error: ' + FAILED_PASSWORD;
    $('#agent-echo-input').value = FAILED_PASSWORD;
    $('#agent-echo-area').value = 'debug: ' + FAILED_PASSWORD;

    const hit = kpxcAgentMode.scan({ password: FAILED_PASSWORD }, [ other ]);
    kpxcAssert(hit.inText, true, card, 'scan() finds the password in the page text');
    kpxcAssert(hit.fields.length, 2, card, 'scan() finds the input and the textarea holding the password');
    kpxcAssert(hit.fields.includes($('#agent-echo-password')), false, card, 'scan() leaves password fields out');
    kpxcAssert(kpxcAgentMode.scan({ password: 'alice' }, [ other ]).fields.length, 0, card, 'scan() skips the fields it is told to ignore');
    kpxcAssert(kpxcAgentMode.scan({ password: 'alice' }).fields.length, 1, card, 'scan() matches a field value otherwise');
    kpxcAssert(kpxcAgentMode.scan({ password: '', totp: undefined }).inText, false, card, 'scan() ignores empty secrets');
    kpxcAssert(kpxcAgentMode.scan({ password: 'not-in-the-page' }).inText, false, card, 'scan() reports no text hit for an absent secret');

    // The code in the text: not inside a longer run of digits
    $('#agent-echo-text').textContent = 'Order 0123456789 shipped';
    kpxcAssert(kpxcAgentMode.scan({ totp: '123456' }).inText, false, card, 'scan() does not take "123456" inside "0123456789" for the code');
    kpxcAssert(kpxcAgentMode.scan({ password: '123456' }).inText, true, card, 'scan() still matches a password of digits exactly');
    $('#agent-echo-text').textContent = 'code: 123456.';
    kpxcAssert(kpxcAgentMode.scan({ totp: '123456' }).inText, true, card, 'scan() finds "123456" in "code: 123456."');
    $('#agent-echo-text').textContent = '123456';
    kpxcAssert(kpxcAgentMode.scan({ totp: '123456' }).inText, true, card, 'scan() finds the code when it is the whole text');
    $('#agent-echo-text').textContent = 'Error: ' + FAILED_PASSWORD;
    $('#agent-echo-area').value = 'ref 0123456789';
    kpxcAssert(kpxcAgentMode.scan({ totp: '123456' }).fields.length, 1, card, 'scan() keeps the exact match for the code in field values');
    $('#agent-echo-area').value = 'debug: ' + FAILED_PASSWORD;

    kpxcAgentMode.clear(hit.fields);
    kpxcAssert($('#agent-echo-input').value, '', card, 'clear() empties the input');
    kpxcAssert($('#agent-echo-area').value, '', card, 'clear() empties the textarea');
    kpxcAssert(other.value, 'alice', card, 'clear() leaves other fields alone');

    kpxcAssert(kpxcAgentMode.fieldsWithValue(FAILED_PASSWORD).length, 1, card, 'fieldsWithValue() includes password fields');
    hide('agent-echo');
}

// A fill, then the page echoes the password (no navigation): fields cleared, grant reported.
async function testExposureAfterFill() {
    const card = AgentCards.EXPOSURE;
    reset(true);
    show('agent-echo');

    await kpxc.initCredentialFields();
    kpxc.credentials = [ ENTRY ];
    await kpxc.prepareCredentials();
    kpxcAssert($('#agent-echo-password').value, ENTRY.password, card, 'The password is filled');
    kpxcAssert(browserStub.sentFor('report_exposure').length, 0, card, 'Nothing is reported before the page shows the secret');

    // The agent submits; the page writes the password into its text and two fields
    $('#agent-echo-form').requestSubmit();
    kpxcAssert($('#agent-echo-input').value, ENTRY.password, card, 'The fixture echoed the password');
    await wait(kpxcAgentMode.RESCAN_DELAY_MS * 3);

    kpxcAssert($('#agent-echo-input').value, '', card, 'The input echoing the password is cleared by the re-scan');
    kpxcAssert($('#agent-echo-area').value, '', card, 'The textarea echoing the password is cleared');
    kpxcAssert($('#agent-echo-password').value, ENTRY.password, card, 'The password field itself is left as filled');
    kpxcAssert(browserStub.sentFor('report_exposure').length, 1, card, 'report_exposure is sent once');
    kpxcAssert(browserStub.sentFor('report_exposure')[0]?.args?.[0], document.location.origin, card, 'report_exposure names the page origin, nothing else');
    kpxcAssert(browserStub.sentFor('agent_mode_take_filled').length, 1, card, 'The handoff to the next load is dropped');

    // The secret is gone from the closure: a second echo is not reported again
    $('#agent-echo-input').value = ENTRY.password;
    await kpxcAgentMode.check();
    kpxcAssert(browserStub.sentFor('report_exposure').length, 1, card, 'After a hit the values are dropped, nothing is scanned for');
    hide('agent-echo');
}

// The next load of the same URL with a password field: the echoed password is emptied first.
async function testClearOnFailure() {
    const card = AgentCards.FAILURE;
    reset(true);
    show('agent-failed');

    const handoff = (url) => ({ password: FAILED_PASSWORD, totp: '', url: url, storedAt: Date.now() });

    $('#agent-failed-password').value = FAILED_PASSWORD;
    $('#agent-failed-username').value = FAILED_PASSWORD;
    $('#agent-failed-other').value = 'keep me';
    browserStub.replies.agent_mode_take_filled = handoff(trimURL(document.location.href));
    await kpxcAgentMode.onPageLoad();
    kpxcAssert(browserStub.sentFor('agent_mode_take_filled').length, 1, card, 'The handoff is read once on load');
    kpxcAssert($('#agent-failed-password').value, '', card, 'The password field is emptied');
    kpxcAssert($('#agent-failed-username').value, '', card, 'Every field holding the filled password is emptied');
    kpxcAssert($('#agent-failed-other').value, 'keep me', card, 'Other fields are left alone');
    kpxcAssert(browserStub.sentFor('report_exposure').length, 0, card, 'A cleared form is not an exposure');
    await wait(kpxcAgentMode.RESCAN_DELAY_MS * 2);

    // Another URL: the form is not a failed sign-in, nothing is cleared
    browserStub.reset({ ...DEFAULT_REPLIES, agent_mode_take_filled: handoff('https://example.com/elsewhere') });
    $('#agent-failed-password').value = FAILED_PASSWORD;
    await kpxcAgentMode.onPageLoad();
    kpxcAssert($('#agent-failed-password').value, FAILED_PASSWORD, card, 'Another URL leaves the form alone');
    await wait(kpxcAgentMode.RESCAN_DELAY_MS * 2);

    // The exposure check still runs after a submit: the page shows the password
    browserStub.reset({ ...DEFAULT_REPLIES, agent_mode_take_filled: handoff('https://example.com/elsewhere') });
    $('#agent-failed-other').value = 'Your password ' + FAILED_PASSWORD + ' is wrong';
    await kpxcAgentMode.onPageLoad();
    kpxcAssert($('#agent-failed-other').value, '', card, 'After a submit, a field showing the password is cleared');
    kpxcAssert(browserStub.sentFor('report_exposure').length, 1, card, 'After a submit, a page showing the password ends the grant');

    // Nothing handed over, or agent mode off: nothing happens
    browserStub.reset({ ...DEFAULT_REPLIES });
    $('#agent-failed-password').value = FAILED_PASSWORD;
    await kpxcAgentMode.onPageLoad();
    kpxcAssert($('#agent-failed-password').value, FAILED_PASSWORD, card, 'Without a handoff the form is left alone');

    reset(false);
    browserStub.replies.agent_mode_take_filled = handoff(trimURL(document.location.href));
    $('#agent-failed-password').value = FAILED_PASSWORD;
    await kpxcAgentMode.onPageLoad();
    kpxcAssert(browserStub.sentFor('agent_mode_take_filled').length, 0, card, 'With agent mode off the handoff is not even read');
    kpxcAssert($('#agent-failed-password').value, FAILED_PASSWORD, card, 'With agent mode off nothing is cleared');
    hide('agent-failed');
}

// Agent mode off: upstream's icons, autocomplete, login popup and banner, no automatic fill.
async function testAgentModeOff() {
    const card = AgentCards.OFF;
    reset(false);
    show('agent-login');

    kpxcAssert(kpxcAgentMode.isOn(), false, card, 'isOn() is false');

    await kpxc.initCredentialFields();
    kpxcAssert(kpxc.combinations.length, 2, card, 'The same fields are detected');
    kpxcAssert(iconCount() > 0, true, card, 'The login icons are created');
    kpxcAssert(injectedWrappers().length > 0, true, card, 'The icons are in the page');

    kpxc.credentials = [ ENTRY, SECOND_ENTRY ];
    await kpxc.prepareCredentials();
    kpxcAssert(kpxcUserAutocomplete.autocompleteList.length > 0, true, card, 'The username autocomplete is attached');
    kpxcAssert(browserStub.sentFor('popup_login').length, 1, card, 'The login popup is offered');
    kpxcAssert($('#agent-password').value, '', card, 'Nothing is filled without the user');
    kpxcAssert(browserStub.sentFor('agent_mode_set_filled').length, 0, card, 'No handoff is stored');

    kpxc.databaseState = DatabaseState.UNLOCKED;
    await kpxcBanner.create({ username: 'alice', password: 'x', url: 'https://example.com', usernameExists: false, list: [] });
    kpxcAssert(kpxcBanner.created, true, card, 'The save banner is created');
    await kpxcBanner.destroy();

    await assertInputFields('basic1', 2);
    hide('agent-login');
}

// Run tests
(async () => {
    kpxcAgentMode.RESCAN_DELAY_MS = 50;

    for (const run of [
        testAgentModeOn,
        testRetrievesWithoutAutoRetrieve,
        testScanAndClear,
        testExposureAfterFill,
        testClearOnFailure,
        testAgentModeOff
    ]) {
        try {
            await run();
        } catch (err) {
            createResult(AgentCards.ON, false, `Test failed: ${run.name} threw ${err}`);
        }
        await wait(kpxcAgentMode.RESCAN_DELAY_MS * 2);
    }

    document.body.append(kpxcUI.createElement('div', '', { id: 'agent-mode-done' }));
})();
