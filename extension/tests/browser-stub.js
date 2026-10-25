'use strict';

// Rowel: a stand-in for the WebExtension API on a fixture page. The content
// scripts run on a file:// page with no background script; the polyfill
// throws on such a page and leaves this `browser` in place. Replies are canned
// per action (browserStub.replies) and every message is kept
// (browserStub.sent), so a test can see what the page asked the background.
const browserStub = {
    sent: [],
    replies: {},

    sentFor(action) {
        return browserStub.sent.filter(m => m.action === action);
    },

    reset(replies = {}) {
        browserStub.sent = [];
        browserStub.replies = replies;
    }
};

globalThis.browser = {
    runtime: {
        getURL: (file) => '../' + file,
        sendMessage: async (message) => {
            browserStub.sent.push(message);
            const reply = browserStub.replies[message.action];
            return typeof reply === 'function' ? reply(message) : reply;
        },
        onMessage: {
            addListener() {}
        }
    },
    i18n: {
        getMessage: (key) => key
    }
};
