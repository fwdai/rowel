'use strict';

// Rowel: logins never cross origins inside a tab. The background's per-tab
// cache and iframe check (background/page.js) run on a blank page behind
// stubs: get-logins answers by the URL's origin and the public suffix probe
// (cookies.set) refuses the suffixes. The context menu's attribute fill
// (content/fill.js) runs against a focused input.
import { test, expect } from '@playwright/test';
import path from 'node:path';

// The background's globals, as the page's scripts declare them
declare const page, tabs, kpxc, kpxcFill, asked, filled;

const BACKGROUND = [ 'common/global.js', 'background/tabs.js', 'background/page.js', 'background/grants.js' ];

const BACKGROUND_STUBS = `
    const VAULT = {
        'https://bank.com': [ { login: 'alice', password: 'bank-secret', name: 'Bank', uuid: 'bank-1' } ],
        'https://evilbank.com': [ { login: 'mallory', password: 'evil-secret', name: 'Evil', uuid: 'evil-1' } ]
    };
    const PUBLIC_SUFFIXES = [ 'com', 'co.uk' ];
    const asked = [];
    var browser = {
        contextMenus: { update: async () => {} },
        cookies: {
            set: async ({ domain }) => {
                if (PUBLIC_SUFFIXES.includes(domain)) {
                    throw new Error('public suffix');
                }
                return {};
            },
            remove: async () => {}
        }
    };
    const keepass = {
        retrieveCredentials: async (_tab, [ url ]) => {
            asked.push(url);
            return VAULT[new URL(url).origin] ?? [];
        }
    };
    const credentials = {};
`;

const load = async (browserPage, stubs, scripts) => {
    await browserPage.setContent('<!doctype html><input id="field">');
    await browserPage.addScriptTag({ content: stubs });
    for (const script of scripts) {
        await browserPage.addScriptTag({ path: path.resolve('rowel', script) });
    }
};

test.describe('Per-tab credential cache: one origin\'s logins never reach another', () => {
    test.beforeEach(async ({ page: browserPage }) => {
        await load(browserPage, BACKGROUND_STUBS, BACKGROUND);
        await browserPage.evaluate(() => tabs.createTabEntry(1));
    });

    test('Logins cached for a bank.com frame are not served to the evilbank.com top document', async ({ page: browserPage }) => {
        const result = await browserPage.evaluate(async () => {
            const tab = { id: 1, active: true, url: 'https://evilbank.com/' };
            const frame = { tab, frameId: 3, url: 'https://bank.com/login' };
            const top = { tab, frameId: 0, url: 'https://evilbank.com/' };

            const forFrame = await page.retrieveCredentials(tab, [ frame.url, 'https://bank.com/session', false ], frame);
            const forTop = await page.retrieveCredentials(tab, [ top.url, top.url, false ], top);
            return { forFrame, forTop, asked };
        });

        expect(result.forFrame.map(c => c.uuid)).toEqual([ 'bank-1' ]);
        expect(result.forTop.map(c => c.uuid)).toEqual([ 'evil-1' ]);
        expect(result.asked).toEqual([ 'https://bank.com/login', 'https://evilbank.com/' ]);
    });

    test('A document asking with a URL of another origin gets nothing and nothing is fetched', async ({ page: browserPage }) => {
        const result = await browserPage.evaluate(async () => {
            const tab = { id: 1, active: true, url: 'https://evilbank.com/' };
            const top = { tab, frameId: 0, url: 'https://evilbank.com/' };

            const forced = await page.retrieveCredentials(tab, [ 'https://bank.com/login', '', true ], top);
            const cached = await page.retrieveCredentials(tab, [ 'https://bank.com/login', '', false ], top);
            const noSender = await page.retrieveCredentials(tab, [ 'https://bank.com/login', '', true ], undefined);
            return { forced, cached, noSender, asked };
        });

        expect(result.forced).toEqual([]);
        expect(result.cached).toEqual([]);
        expect(result.noSender).toEqual([]);
        expect(result.asked).toEqual([]);
    });

    test('The same origin is still served from the cache, and a forced retrieval still asks again', async ({ page: browserPage }) => {
        const result = await browserPage.evaluate(async () => {
            const tab = { id: 1, active: true, url: 'https://bank.com/' };
            const top = { tab, frameId: 0, url: 'https://bank.com/' };
            const frame = { tab, frameId: 3, url: 'https://bank.com/login' };

            const first = await page.retrieveCredentials(tab, [ top.url, top.url, false ], top);
            const sameOriginFrame = await page.retrieveCredentials(tab, [ frame.url, frame.url, false ], frame);
            const askedBeforeForce = asked.length;
            const forced = await page.retrieveCredentials(tab, [ frame.url, frame.url, true ], frame);
            return { first, sameOriginFrame, askedBeforeForce, forced, asked };
        });

        expect(result.first.map(c => c.uuid)).toEqual([ 'bank-1' ]);
        expect(result.sameOriginFrame.map(c => c.uuid)).toEqual([ 'bank-1' ]);
        expect(result.askedBeforeForce).toBe(1);
        expect(result.forced.map(c => c.uuid)).toEqual([ 'bank-1' ]);
        expect(result.asked).toEqual([ 'https://bank.com/', 'https://bank.com/login' ]);
    });

    test('The single cached login\'s uuid goes only to a document of its origin', async ({ page: browserPage }) => {
        const result = await browserPage.evaluate(async () => {
            const tab = { id: 1, active: true, url: 'https://evilbank.com/' };
            const frame = { tab, frameId: 3, url: 'https://bank.com/login' };
            const top = { tab, frameId: 0, url: 'https://evilbank.com/' };

            await page.retrieveCredentials(tab, [ frame.url, frame.url, false ], frame);
            return {
                forTop: await page.getLoginId(tab, undefined, top),
                forFrame: await page.getLoginId(tab, undefined, frame)
            };
        });

        expect(result.forTop).toBeUndefined();
        expect(result.forFrame).toBe('bank-1');
    });
});

test.describe('Iframe check: the frame and the tab share the base domain on a label boundary', () => {
    test.beforeEach(async ({ page: browserPage }) => {
        await load(browserPage, BACKGROUND_STUBS, BACKGROUND);
    });

    const cases: [ string, string, boolean ][] = [
        [ 'https://bank.com/login', 'https://evilbank.com/', false ],
        [ 'https://ypal.com/login', 'https://paypal.com/', false ],
        [ 'https://login.example.com/', 'https://www.example.com/', true ],
        [ 'https://login.example.com/', 'https://example.com/', true ],
        [ 'https://example.com:8443/', 'https://example.com/', true ],
        [ 'https://login.example.co.uk/', 'https://www.example.co.uk/', true ]
    ];

    for (const [ frameUrl, tabUrl, expected ] of cases) {
        test(`${new URL(frameUrl).hostname} framed in ${new URL(tabUrl).hostname}: ${expected ? 'allowed' : 'refused'}`, async ({ page: browserPage }) => {
            const allowed = await browserPage.evaluate(([ frame, top ]) => {
                return page.isIframeAllowed({ id: 1, url: top }, [ frame, new URL(frame).hostname ]);
            }, [ frameUrl, tabUrl ]);

            expect(allowed).toBe(expected);
        });
    }

    test('The site preference "allow iframes" still allows any frame', async ({ page: browserPage }) => {
        const allowed = await browserPage.evaluate(async () => {
            await tabs.createTabEntry(2);
            tabs.updateTabValues(2, { allowIframes: true });
            return page.isIframeAllowed({ id: 2, url: 'https://evilbank.com/' }, [ 'https://bank.com/login', 'bank.com' ]);
        });

        expect(allowed).toBe(true);
    });
});

test.describe('Context menu attribute fill: only an attribute of the frame\'s own logins', () => {
    test.beforeEach(async ({ page: browserPage }) => {
        await load(browserPage, `
            const filled = [];
            const kpxc = {
                credentials: [ { login: 'mallory', uuid: 'evil-1', stringFields: [ { 'KPH: pin': '0000' } ] } ]
            };
        `, [ 'content/fill.js' ]);
        await browserPage.evaluate(() => {
            kpxcFill.setValue = async (_field, value) => filled.push(value);
            document.getElementById('field').focus();
        });
    });

    test('Another origin\'s attribute is not filled', async ({ page: browserPage }) => {
        const result = await browserPage.evaluate(async () => {
            await kpxcFill.fillAttributeToActiveElementWith({ 'KPH: pin': '1234' });
            return filled;
        });

        expect(result).toEqual([]);
    });

    test('The frame\'s own attribute is filled', async ({ page: browserPage }) => {
        const result = await browserPage.evaluate(async () => {
            await kpxcFill.fillAttributeToActiveElementWith({ 'KPH: pin': '0000' });
            return filled;
        });

        expect(result).toEqual([ '0000' ]);
    });
});
