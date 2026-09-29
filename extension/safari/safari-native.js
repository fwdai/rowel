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
// Ordering holds by construction. Every native message goes through one
// promise chain, so there is only ever one outstanding and responses are
// handled in the order they were sent, whatever order Safari delivers them in.
// The handler serves one at a time too and lists each response's events —
// signals and the reply — in the order the app sent them, so they are emitted
// as listed. `after` (the last signal before the reply) is only checked.
//
// Signals come in on replies and on a heartbeat: every two seconds, when
// nothing is outstanding, the port polls the handler (Safari does not let the
// handler speak first). A long request — a consent dialog can take a minute —
// holds the heartbeat off rather than queueing polls behind it. The handler
// also pushes each signal to a real native port when Safari lets it (Apple
// does not document that from inside an extension); a push is taken only as
// a cue to poll now, so it cannot reorder anything.

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
            this.chain = Promise.resolve();
            this.outstanding = 0; // messages queued or on the way
            this.pollQueued = false;
            this.heartbeat = undefined;
            this.pushed = undefined;

            try {
                this.pushed = connectNative(name);
                this.pushed.onMessage.addListener(() => this.poll());
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
            this.enqueue({ request: JSON.stringify(request) });
        }

        disconnect() {
            // The caller's own disconnect: no onDisconnect, as with any port.
            this.close();
        }

        // A poll, unless one is already waiting its turn.
        poll() {
            if (this.open && !this.pollQueued) {
                this.pollQueued = true;
                this.enqueue({ poll: true });
            }
        }

        // One native message at a time, handled in the order sent.
        enqueue(envelope) {
            this.outstanding++;
            const run = async () => {
                if (envelope.poll) {
                    this.pollQueued = false;
                }
                try {
                    await this.exchange(envelope);
                } finally {
                    this.outstanding--;
                }
            };
            this.chain = this.chain.then(run, run);
        }

        async exchange(envelope) {
            if (!this.open) {
                return;
            }
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
            this.handle(response);
        }

        handle(response) {
            const events = Array.isArray(response.events) ? response.events : [];
            const after = typeof response.after === 'number' ? response.after : undefined;
            let replied = false;
            for (const event of events) {
                if (typeof event?.reply === 'string') {
                    replied = true;
                    const reply = parse(event.reply);
                    if (reply) {
                        this.live = true;
                        this.startHeartbeat();
                        this.onMessage.emit(reply, this);
                    }
                } else if (typeof event?.seq === 'number') {
                    if (after !== undefined && (replied ? event.seq <= after : event.seq > after)) {
                        console.warn(`Rowel: signal ${event.seq} is ${replied ? 'after' : 'before'} `
                            + `the reply, but the app sent it ${replied ? 'before' : 'after'} (after=${after})`);
                    }
                    this.signal(event);
                }
            }

            if (response.error) {
                this.fail(`${response.error}: ${response.detail ?? ''}`);
            } else if (this.live && response.connected === false) {
                // The connection is gone: the app quit, or switched the browser
                // integration off.
                this.fail('the Rowel app closed the connection');
            }
        }

        signal(event) {
            if (event.seq <= this.delivered) {
                return;
            }
            this.delivered = event.seq;
            const message = parse(event.signal);
            if (message) {
                this.onMessage.emit(message, this);
            }
        }

        startHeartbeat() {
            if (this.heartbeat === undefined) {
                this.heartbeat = setInterval(() => {
                    if (this.outstanding === 0) {
                        this.poll();
                    }
                }, HEARTBEAT_MS);
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
