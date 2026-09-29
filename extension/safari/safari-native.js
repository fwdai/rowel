'use strict';

// Rowel: native messaging under Safari.
//
// Safari has no native messaging host to launch. Messages go to the handler in
// the app extension shipped inside Rowel.app (SafariWebExtensionHandler, in
// src-tauri/gen/safari), one request and one reply at a time, whatever host
// name is asked for. That handler holds the connection to the app, and hands
// back what the app sent unsolicited — the database-locked/-unlocked signals —
// with its replies.
//
// This replaces browser.runtime.connectNative with a port that behaves the way
// background/client.js expects a Chrome or Firefox one to: postMessage() sends
// a request, every reply and signal arrives through onMessage in the order the
// app sent them, and onDisconnect fires when the app is not there or has gone —
// the "not connected" state, and upstream's automatic reconnect, exactly as a
// proxy that exits does it. Nothing upstream is changed; this file is loaded
// only by the Safari manifest, right after the polyfill (safari/build.mjs).
//
// Signals reach the port three ways, deduplicated by the sequence number the
// handler gives each one: on a reply; on the heartbeat below, which asks the
// handler for them while nothing else is being sent (Safari does not let the
// handler speak first); and pushed to a real native port, if Safari delivers
// what the handler dispatches from inside the extension process, which Apple
// does not document.

(function() {
    const HEARTBEAT_MS = 2000;

    const runtime = browser.runtime;
    const sendNativeMessage = runtime.sendNativeMessage.bind(runtime);
    const connectNative = runtime.connectNative.bind(runtime);

    const listeners = function() {
        const set = new Set();
        return {
            addListener: (fn) => set.add(fn),
            removeListener: (fn) => set.delete(fn),
            hasListener: (fn) => set.has(fn),
            emit: (...args) => {
                for (const fn of [ ...set ]) {
                    try {
                        fn(...args);
                    } catch (e) {
                        console.error('Rowel: port listener failed', e);
                    }
                }
            },
        };
    };

    const parse = function(text) {
        try {
            return JSON.parse(text);
        } catch (_e) {
            return undefined;
        }
    };

    class SafariNativePort {
        constructor(name) {
            this.name = name;
            this.onMessage = listeners();
            this.onDisconnect = listeners();
            this.open = true;
            this.live = false; // a request has been answered by the app
            this.delivered = 0; // the last signal handed to onMessage
            this.heartbeat = undefined;
            this.pushed = undefined;

            try {
                this.pushed = connectNative(name);
                this.pushed.onMessage.addListener((m) => {
                    const signal = m?.userInfo ?? m;
                    if (signal && typeof signal.seq === 'number') {
                        this.signals([ signal ], Infinity);
                    }
                });
                this.pushed.onDisconnect.addListener(() => {
                    this.pushed = undefined;
                });
            } catch (_e) {
                this.pushed = undefined;
            }
        }

        postMessage(request) {
            if (!this.open) {
                throw new Error('Attempt to postMessage on disconnected port');
            }
            this.ask({ request: JSON.stringify(request) });
        }

        disconnect() {
            // The caller's own disconnect: no onDisconnect, as with any port.
            this.close();
        }

        async ask(envelope) {
            let response;
            try {
                response = await sendNativeMessage(this.name, envelope);
            } catch (e) {
                this.fail(e?.message ?? String(e));
                return;
            }
            if (!this.open) {
                return;
            }
            if (!response || typeof response !== 'object') {
                this.fail('no response from the Rowel app extension');
                return;
            }

            const signals = Array.isArray(response.signals) ? response.signals : [];
            if (typeof response.reply === 'string') {
                const after = typeof response.after === 'number' ? response.after : 0;
                this.signals(signals, after);
                const reply = parse(response.reply);
                if (reply) {
                    this.live = true;
                    this.startHeartbeat();
                    this.onMessage.emit(reply, this);
                }
                this.signals(signals, Infinity);
                return;
            }

            this.signals(signals, Infinity);
            if (response.error) {
                this.fail(`${response.error}: ${response.detail ?? ''}`);
            } else if (this.live && response.connected === false) {
                // A heartbeat that found the connection gone: the app quit,
                // or switched the browser integration off.
                this.fail('the Rowel app closed the connection');
            }
        }

        // Hand onMessage the signals up to `upTo` not yet handed over, oldest
        // first.
        signals(list, upTo) {
            for (const signal of [ ...list ].sort((a, b) => a.seq - b.seq)) {
                if (signal.seq > upTo || signal.seq <= this.delivered) {
                    continue;
                }
                this.delivered = signal.seq;
                const message = typeof signal.message === 'string' ? parse(signal.message) : signal.message;
                if (message) {
                    this.onMessage.emit(message, this);
                }
            }
        }

        startHeartbeat() {
            if (this.heartbeat === undefined) {
                this.heartbeat = setInterval(() => this.ask({ poll: true }), HEARTBEAT_MS);
            }
        }

        fail(reason) {
            if (!this.open) {
                return;
            }
            console.log(`Rowel: native messaging disconnected (${reason})`);
            this.close();
            this.onDisconnect.emit(this);
        }

        close() {
            this.open = false;
            clearInterval(this.heartbeat);
            this.heartbeat = undefined;
            try {
                this.pushed?.disconnect();
            } catch (_e) {
                // Already gone.
            }
            this.pushed = undefined;
        }
    }

    const safariConnectNative = (name) => new SafariNativePort(name);

    // The polyfill may have wrapped the runtime in a Proxy that answers from its
    // own cache: a definition lands there, where an assignment might not.
    Object.defineProperty(runtime, 'connectNative', {
        value: safariConnectNative,
        configurable: true,
        writable: true,
    });
    if (browser.runtime.connectNative !== safariConnectNative) {
        console.error('Rowel: could not install the Safari native messaging port');
    }
})();
