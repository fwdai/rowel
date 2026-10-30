'use strict';

// Rowel: the popup's locked and not-running screens (popups/app-status.js),
// on the real popup pages behind a stand-in for the WebExtension API, served
// from rowel/ the way agent-mode.spec.ts serves its fixture. The stand-in is
// in place before any page script, so upstream's popup scripts, agent-mode.js
// and app-status.js all run as they do in the extension, against canned
// replies; i18n answers with the key.
//
// Run with the other extension tests (`bun run extension:test`).
import { test, expect, Page } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';

const ORIGIN = 'https://rowel.test';
const ROOT = path.resolve('rowel');

// Statuses as kpxcEvent.showStatus gives them.
const LOCKED = { keePassXCAvailable: true, databaseClosed: true, configured: true, error: 'Database not opened' };
const NOT_RUNNING = { keePassXCAvailable: false, databaseClosed: true, configured: false, error: 'errorMessageTimeout' };
const CONNECTED = {
    keePassXCAvailable: true,
    databaseClosed: false,
    configured: true,
    associated: true,
    identifier: 'laptop'
};

const BASE_REPLIES = {
    get_color_theme: 'light',
    update_available_keepassxc: false,
    load_settings: {}
};

// `replies` maps an action to its reply. The background's replies come back
// once the page's scripts have run, as they do in the extension; `early`
// answers at once instead, before the deferred scripts are all in.
async function openPopup(page: Page, file: string, replies: Record<string, unknown>, early = false) {
    await page.route(`${ORIGIN}/**`, (route) => {
        const file = path.join(ROOT, decodeURIComponent(new URL(route.request().url()).pathname));
        return fs.existsSync(file) ? route.fulfill({ path: file }) : route.fulfill({ status: 404 });
    });
    await page.addInitScript(({ replies, early }) => {
        const sent: { action: string, args?: unknown }[] = [];
        const loaded = new Promise((resolve) => document.addEventListener('DOMContentLoaded', resolve));
        Object.assign(window, { rowelSent: sent, rowelClosed: false });
        window.close = () => Object.assign(window, { rowelClosed: true });
        Object.assign(globalThis, {
            browser: {
                runtime: {
                    getURL: (resource: string) => '../' + resource,
                    sendMessage: async (message: { action: string }) => {
                        sent.push(message);
                        if (!early) {
                            await loaded;
                        }
                        return replies[message.action];
                    },
                    openOptionsPage: async () => {},
                    onMessage: { addListener() {} }
                },
                tabs: {
                    query: async () => [ { id: 1 } ],
                    sendMessage: async () => {}
                },
                // The key, and the substitutions after it: translate.js puts
                // an element in through one (#associated-identifier).
                i18n: {
                    getMessage: (key: string, substitutions?: string | string[]) =>
                        [ key, ...[ substitutions ?? [] ].flat() ].join(' ')
                }
            }
        });
    }, { replies: { ...BASE_REPLIES, ...replies }, early });
    await page.goto(`${ORIGIN}/popups/${file}`);
}

const sent = (page: Page, action: string) =>
    page.evaluate((action) => (window as any).rowelSent.filter((m: { action: string }) => m.action === action), action);

test.describe('Popup: locked and not-running screens', () => {
    test('Locked: its screen stands in for upstream\'s error, and Unlock asks the host to bring Rowel forward', async ({ page }) => {
        await openPopup(page, 'popup.html', { get_status: LOCKED });

        await expect(page.locator('#rowel-locked')).toBeVisible();
        await expect(page.locator('#rowel-locked .rowel-status-title')).toHaveText('rowelLockedTitle');
        await expect(page.locator('#database-not-opened')).toBeHidden();
        await expect(page.locator('#rowel-agent-mode')).toBeHidden();
        await expect(page.locator('#rowel-not-running')).toBeHidden();

        await page.locator('#rowel-locked button').click();
        await expect.poll(async () => (await sent(page, 'get_status')).map((m: { args?: unknown }) => m.args))
            .toContainEqual([ false, true ]);
        await expect.poll(() => page.evaluate(() => (window as any).rowelClosed)).toBe(true);
    });

    test('Locked by the popup\'s own lock button: the extension\'s text for it is the locked state too', async ({ page }) => {
        await openPopup(page, 'popup.html', {
            get_status: CONNECTED,
            lock_database: { ...LOCKED, error: 'errorMessageDatabaseNotOpened' }
        });

        await expect(page.locator('#configured-and-associated')).toBeVisible();
        await page.locator('#lock-database-button').click();
        await expect(page.locator('#rowel-locked')).toBeVisible();
        await expect(page.locator('#configured-and-associated')).toBeHidden();
    });

    test('Another error beside a closed database is still upstream\'s error block', async ({ page }) => {
        await openPopup(page, 'popup.html', { get_status: { ...LOCKED, error: 'Cannot decrypt message' } });

        await expect(page.locator('#database-not-opened')).toBeVisible();
        await expect(page.locator('#database-error-message')).toHaveText('Cannot decrypt message');
        await expect(page.locator('#rowel-locked')).toBeHidden();
    });

    test('Not running: one quiet reconnect, then its screen; Try again looks again', async ({ page }) => {
        await openPopup(page, 'popup.html', { get_status: NOT_RUNNING, reconnect: NOT_RUNNING });

        await expect(page.locator('#rowel-not-running')).toBeVisible();
        await expect(page.locator('#rowel-not-running .rowel-status-title')).toHaveText('rowelNotRunningTitle');
        // "Open the Rowel app and [Try again]": the button finishes the line.
        await expect(page.locator('#rowel-not-running .rowel-status-text + button'))
            .toHaveText('rowelTryAgainButton');
        await expect(page.locator('#error-encountered')).toBeHidden();
        await expect(page.locator('#rowel-agent-mode')).toBeHidden();
        expect(await sent(page, 'reconnect')).toHaveLength(1);

        await page.locator('#rowel-not-running button').click();
        await expect.poll(async () => (await sent(page, 'reconnect')).length).toBe(2);
        await expect(page.locator('#rowel-not-running')).toBeVisible();
        await expect(page.locator('#initial-state')).toBeHidden();
    });

    test('Not running when the popup opened, but Rowel was opened since: the quiet reconnect finds it locked', async ({ page }) => {
        await openPopup(page, 'popup.html', { get_status: NOT_RUNNING, reconnect: LOCKED });

        await expect(page.locator('#rowel-locked')).toBeVisible();
        await expect(page.locator('#rowel-not-running')).toBeHidden();
        await expect(page.locator('#error-encountered')).toBeHidden();
    });

    test('A first status that reached upstream before the override loaded is rendered again through it', async ({ page }) => {
        await openPopup(page, 'popup.html', { get_status: NOT_RUNNING, reconnect: NOT_RUNNING }, true);

        await expect(page.locator('#rowel-not-running')).toBeVisible();
        await expect(page.locator('#error-encountered')).toBeHidden();
        expect(await sent(page, 'reconnect')).toHaveLength(1);
    });

    test('Connected: neither screen', async ({ page }) => {
        await openPopup(page, 'popup.html', { get_status: CONNECTED });

        await expect(page.locator('#configured-and-associated')).toBeVisible();
        await expect(page.locator('#rowel-locked')).toBeHidden();
        await expect(page.locator('#rowel-not-running')).toBeHidden();
        expect(await sent(page, 'reconnect')).toHaveLength(0);
    });

    test('Agent mode on: its screen wins over the locked one', async ({ page }) => {
        await openPopup(page, 'popup.html', { get_status: LOCKED, load_settings: { agentMode: true } });

        await expect(page.locator('#rowel-agent-mode-on')).toBeVisible();
        await expect(page.locator('#rowel-locked')).toBeHidden();
        await expect(page.locator('#database-not-opened')).toBeHidden();
    });

    test('The login list\'s lock button shows the locked screen', async ({ page }) => {
        await openPopup(page, 'popup_login.html', { get_login_list: [] });
        // Upstream wires its lock button once the list has come back.
        await expect.poll(async () => (await sent(page, 'get_login_list')).length).toBe(1);

        await page.locator('#lock-database-button').click();
        await expect(page.locator('#rowel-locked')).toBeVisible();
        await expect(page.locator('#database-not-opened')).toBeHidden();
        expect(await sent(page, 'lock_database')).toHaveLength(1);

        await page.locator('#rowel-locked button').click();
        await expect.poll(async () => (await sent(page, 'get_status')).map((m: { args?: unknown }) => m.args))
            .toContainEqual([ false, true ]);
    });

    test('The HTTP auth list\'s lock button shows the locked screen', async ({ page }) => {
        await openPopup(page, 'popup_httpauth.html', { get_login_list: { loginList: [] } });
        await expect.poll(async () => (await sent(page, 'get_login_list')).length).toBe(1);

        await page.locator('#lock-database-button').click();
        await expect(page.locator('#rowel-locked')).toBeVisible();
        await expect(page.locator('#database-not-opened')).toBeHidden();
        expect(await sent(page, 'lock_database')).toHaveLength(1);
    });
});
