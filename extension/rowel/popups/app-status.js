'use strict';

/* global statusResponse: writable */

// Rowel: the popup's two everyday states as screens of their own. Upstream
// shows both under "Rowel has encountered an error:" with the raw reason (for
// a locked Rowel, the host's "Database not opened"), but neither is a fault.
// A locked Rowel serves nothing by design, so the popup says to unlock it; one
// that isn't running is just closed, so the popup says to open it.
//
// Loaded after the page's own script by popup.html, popup_login.html and
// popup_httpauth.html. Upstream's markup and scripts stay as they are
// (docs/browser-extension.md): its status logic still shows and hides
// #database-not-opened and #error-encountered, the screens below are added at
// the end of `.container`, and a body class puts one in place of upstream's
// block (popup.css). Their buttons click upstream's, so what they do stays
// upstream's.
(() => {
    const LOCKED = 'rowel-locked';
    const NOT_RUNNING = 'rowel-not-running';

    // The `error` upstream gives a closed database when Rowel is locked: its
    // own copy for code 1, or the host's text for that code as it came over
    // the wire (src-tauri/crates/broker/src/protocol.rs). Any other error
    // beside a closed database is a real one, and upstream's block shows it.
    const LOCKED_ERRORS = [ undefined, '', tr('errorMessageDatabaseNotOpened'), 'Database not opened' ];

    const stateOf = function(status) {
        if (!status) {
            return null;
        }
        if (!status.keePassXCAvailable) {
            return NOT_RUNNING;
        }
        if (status.databaseClosed && LOCKED_ERRORS.includes(status.error)) {
            return LOCKED;
        }
        return null;
    };

    const render = function(state) {
        document.body.classList.toggle(LOCKED, state === LOCKED);
        document.body.classList.toggle(NOT_RUNNING, state === NOT_RUNNING);
    };

    const element = function(tag, className, text) {
        const node = document.createElement(tag);
        node.className = className;
        if (text) {
            node.textContent = text;
        }
        return node;
    };

    // Icon, title, the step to take, the button that takes or follows it, and
    // a hint under the button.
    const screen = function(id, glyph, title, text, action, hint) {
        const root = element('div', 'rowel-status');
        root.id = id;
        root.setAttribute('role', 'status');

        const icon = element('div', 'rowel-status-icon');
        const glyphNode = element('i', `fa ${glyph}`);
        glyphNode.setAttribute('aria-hidden', 'true');
        icon.append(glyphNode);

        root.append(icon, element('p', 'rowel-status-title', title), element('p', 'rowel-status-text', text));
        if (action) {
            root.append(action);
        }
        if (hint) {
            root.append(element('p', 'rowel-status-hint', hint));
        }
        return root;
    };

    // A button that clicks upstream's own, hidden under the screen.
    const proxyButton = function(className, text, title, upstreamId, before) {
        const upstream = document.getElementById(upstreamId);
        if (!upstream) {
            return null;
        }
        const button = element('button', className, text);
        button.type = 'button';
        if (title) {
            button.title = title;
        }
        button.addEventListener('click', () => {
            before?.();
            upstream.click();
        });
        return button;
    };

    const container = document.querySelector('.container');

    // Unlocking happens in the app: upstream's reopen button asks the host to
    // bring Rowel's window forward (`triggerUnlock`), and the browser closes
    // the popup as Rowel takes focus.
    container.append(screen(
        LOCKED,
        'fa-lock',
        tr('rowelLockedTitle'),
        tr('rowelLockedText'),
        proxyButton('btn btn-primary', tr('rowelUnlockButton'), tr('rowelUnlockButtonTitle'), 'reopen-database-button')
    ));

    if (typeof statusResponse !== 'function') {
        // popup_login.html and popup_httpauth.html show the locked block only
        // from their own lock button, and never the not-running one.
        document.getElementById('lock-database-button')?.addEventListener('click', () => render(LOCKED));
        return;
    }

    // The extension cannot start Rowel (the native host it launches only
    // relays to a running app), so the screen reads as one sentence the button
    // finishes: "Open the Rowel app and [Try again]". Trying again is
    // upstream's reload, under upstream's "Checking status…".
    const checking = function() {
        render(null);
        document.getElementById('error-encountered').style.display = 'none';
        document.getElementById('initial-state').style.display = 'block';
    };
    container.append(screen(
        NOT_RUNNING,
        'fa-power-off',
        tr('rowelNotRunningTitle'),
        tr('rowelNotRunningText'),
        proxyButton('btn btn-primary', tr('rowelTryAgainButton'), null, 'reload-status-button', checking),
        tr('rowelNotRunningHint')
    ));

    // popup.html renders every status through upstream's statusResponse. The
    // extension does not reconnect on its own (autoReconnect is off by
    // default), so a Rowel opened since it last looked would still read as not
    // running: the first time a status says so, reconnect once, quietly, before
    // showing anything.
    const upstreamStatusResponse = statusResponse;
    let wrapped = false;
    let reconnected = false;
    statusResponse = function(status) {
        wrapped = true;
        const state = stateOf(status);
        if (state === NOT_RUNNING && !reconnected) {
            reconnected = true;
            browser.runtime.sendMessage({ action: 'reconnect' }).then(
                (next) => statusResponse(next ?? status),
                () => statusResponse(status)
            );
            return;
        }

        upstreamStatusResponse(status);
        render(state);
    };

    // popup.js asks for the first status while the deferred scripts are still
    // loading, and a reply that lands before this one runs goes to upstream's
    // statusResponse unwrapped. Upstream hides #initial-state when it renders:
    // if that happens before the wrapper has seen a status, read it again (an
    // internal poll, which asks the host nothing) and render it through here.
    const initialState = document.getElementById('initial-state');
    const readAgain = function() {
        catchUp.disconnect();
        browser.runtime.sendMessage({ action: 'get_status', args: [ true ] }).then((status) => {
            if (status) {
                statusResponse(status);
            }
        });
    };
    const catchUp = new MutationObserver(() => {
        if (wrapped) {
            catchUp.disconnect();
        } else if (initialState.style.display === 'none') {
            readAgain();
        }
    });
    if (initialState.style.display === 'none') {
        readAgain();
    } else {
        catchUp.observe(initialState, { attributes: true, attributeFilter: [ 'style' ] });
    }
})();
