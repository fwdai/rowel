'use strict';

// Rowel: logins never cross origins inside a tab. The background's per-tab
// cache and iframe check (background/page.js) run on a blank page behind
// stubs: get-logins answers by the URL's origin and the public suffix probe
// (cookies.set) refuses the suffixes. The context menu's attribute fill
// (content/fill.js) runs against a focused input.
import { test, expect } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';

// The background's globals, as the page's scripts declare them
declare const page, tabs, kpxc, kpxcFill, kpxcBanner, browserStub, DatabaseState, asked, filled;

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

    test('A login selected in one origin\'s frame is not told to another origin, which keeps its own', async ({ page: browserPage }) => {
        const result = await browserPage.evaluate(async () => {
            const tab = { id: 1, active: true, url: 'https://evilbank.com/' };
            const frame = { tab, frameId: 3, url: 'https://bank.com/login' };
            const top = { tab, frameId: 0, url: 'https://evilbank.com/' };

            await page.retrieveCredentials(tab, [ frame.url, frame.url, false ], frame);
            await page.setLoginId(tab, 'bank-1', frame);
            const forFrame = await page.getLoginId(tab, false, frame);
            await page.retrieveCredentials(tab, [ top.url, top.url, false ], top);
            return {
                forFrame,
                forTop: await page.getLoginId(tab, undefined, top),
                forTopUnselected: await page.getLoginId(tab, false, top)
            };
        });

        expect(result.forFrame).toBe('bank-1');
        expect(result.forTop).toBe('evil-1');
        expect(result.forTopUnselected).toBeUndefined();
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

// The update list of the save banner, on the content-script fixture agent-mode.spec.ts uses.
// retrieve_credentials is answered by the background's rule: the fresh list for a URL of the
// asking document's origin, nothing for any other.
test.describe('Update banner: the count check is for the origin that submitted', () => {
    const ORIGIN = 'https://rowel.test';
    const LONG_ORIGIN = `https://${'a'.repeat(50)}.rowel.test`;
    const ROOT = path.resolve('rowel');

    // Two logins to choose from, `fresh` of them still in the vault; the first one is clicked.
    const updatesAfterChoosing = async (browserPage, origin, url, fresh) => {
        await browserPage.route(`${origin}/**`, (route) => {
            const file = path.join(ROOT, decodeURIComponent(new URL(route.request().url()).pathname));
            return fs.existsSync(file) ? route.fulfill({ path: file }) : route.fulfill({ status: 404 });
        });
        await browserPage.goto(`${origin}/tests/agent-mode.html`);
        await browserPage.waitForSelector('#agent-mode-done', { state: 'attached' });

        await browserPage.evaluate(async ([ submittedUrl, freshCount ]) => {
            await kpxcBanner.destroy();
            kpxc.settings = { ...kpxc.settings, agentMode: false, showLoginNotifications: true, sitePreferences: [] };
            kpxc.databaseState = DatabaseState.UNLOCKED;
            const list = [ { login: 'alice', name: 'One', uuid: 'u1' }, { login: 'alice', name: 'Two', uuid: 'u2' } ];
            browserStub.reset({
                banner_get_position: 1,
                get_connected_database: { count: 1, identifier: 'test' },
                retrieve_credentials: ({ args: [ asked ] }) =>
                    (new URL(asked).origin === window.location.origin ? list.slice(0, freshCount) : []),
                update_credentials: 'error'
            });
            const submitted = { username: 'alice', password: 'new-password', url: submittedUrl, usernameExists: true, list };
            await kpxcBanner.create(submitted);
            await kpxcBanner.updateCredentials(submitted);
        }, [ url, fresh ]);

        // The list is in a closed shadow root and its links answer only a trusted click.
        const middle = () => browserPage.evaluate(() => {
            const rect = kpxcBanner.shadowSelector('ul#list a').getBoundingClientRect();
            return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2, shown: rect.width > 0 };
        });
        await expect.poll(async () => (await middle()).shown).toBe(true);
        const link = await middle();

        await browserPage.mouse.click(link.x, link.y);
        await expect.poll(() => browserPage.evaluate(() => browserStub.sentFor('retrieve_credentials').length)).toBe(1);
        await browserPage.waitForTimeout(200);
        return browserPage.evaluate(() => browserStub.sentFor('update_credentials').length);
    };

    // The submitted URL as the page saves it: the origin alone ("save domain only") or the page's URL.
    for (const [ name, origin ] of [ [ 'a short origin', ORIGIN ], [ 'an origin over 50 characters', LONG_ORIGIN ] ]) {
        for (const [ saved, url ] of [ [ 'the origin alone', origin ], [ 'a page URL', `${origin}/${'login/'.repeat(12)}` ] ]) {
            test(`On ${name}, saved as ${saved}: an unchanged list lets the update through`, async ({ page: browserPage }) => {
                expect(await updatesAfterChoosing(browserPage, origin, url, 2)).toBe(1);
            });

            test(`On ${name}, saved as ${saved}: a changed list stops the update`, async ({ page: browserPage }) => {
                expect(await updatesAfterChoosing(browserPage, origin, url, 1)).toBe(0);
            });
        }
    }

    test('After a redirect to another origin the update goes through', async ({ page: browserPage }) => {
        expect(await updatesAfterChoosing(browserPage, ORIGIN, 'https://login.other.test/session', 2)).toBe(1);
    });
});
