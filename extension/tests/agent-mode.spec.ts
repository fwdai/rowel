'use strict';

// Rowel: agent mode in the content scripts (content/agent-mode.js), run
// against tests/agent-mode.html the way content-scripts.spec.ts runs
// tests.html. The checks themselves are in agent-mode-tests.js; this reads
// their result cards once the page has finished.
import { test, expect } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';

// The fixture is served from rowel/ under a real origin rather than file://:
// the retrieval path goes through upstream's isIframeAllowed(), which treats an
// opaque origin as a sandboxed frame, and Firefox gives file:// pages one.
const ORIGIN = 'https://rowel.test';
const ROOT = path.resolve('rowel');

let page;

test.describe('Agent mode content script tests', () => {
    test.beforeAll(async ({ browser }) => {
        page = await browser.newPage();
        await page.route(`${ORIGIN}/**`, (route) => {
            const file = path.join(ROOT, decodeURIComponent(new URL(route.request().url()).pathname));
            return fs.existsSync(file) ? route.fulfill({ path: file }) : route.fulfill({ status: 404 });
        });
        await page.goto(`${ORIGIN}/tests/agent-mode.html`);
        await page.waitForSelector('#agent-mode-done', { state: 'attached' });
    });

    test('Agent mode on: no icons, autocomplete or banners, the admitted entry is filled', async () => {
        await verifyResults('agent-on-results');
    });

    test('Agent mode retrieves on its own with autoRetrieveCredentials off', async () => {
        await verifyResults('agent-retrieve-results');
    });

    test('Denied fill: the page gets nothing filled and no UI', async () => {
        await verifyResults('agent-denied-results');
    });

    test('Exposure scan finds the secret in text and fields, clear empties them', async () => {
        await verifyResults('agent-scan-results');
    });

    test('Exposure after fill: echoed password is cleared and the grant reported', async () => {
        await verifyResults('agent-exposure-results');
    });

    test('Clear on failure: the same URL with a password field is cleared before anything else', async () => {
        await verifyResults('agent-failure-results');
    });

    test('One fill\'s state does not reach the next: the old TOTP element is scanned again', async () => {
        await verifyResults('agent-fill-state-results');
    });

    test('A fill ends when the next begins: finalised with its own scan, never sharing state', async () => {
        await verifyResults('agent-lifecycle-results');
    });

    test('Agent mode off: upstream behaviour is unchanged', async () => {
        await verifyResults('agent-off-results');
    });

    test('Input field matching is the same in both modes', async () => {
        await verifyResults('input-field-results');
    });
});

const verifyResults = async(selector) => {
    const resultCount = await page.locator(`css=#${selector} >> css=.fa`).count();
    await expect.soft(resultCount).toBeGreaterThan(0);

    for (let i = 0; i < resultCount; i++) {
        const elem = await page.locator(`css=#${selector} >> css=.fa`).nth(i);
        const id = await elem.getAttribute('id');
        await expect.soft(elem, id).toHaveClass('fa fa-check');
    }
};
