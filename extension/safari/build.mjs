#!/usr/bin/env node
// Rowel: the Safari flavour of the extension, unpacked into build/safari/ for
// scripts/build-safari-extension.sh to put in the Safari app extension's
// Resources. `npm run build:safari` (or `bun run build:safari`); no
// dependencies, so it needs no install. Run it after `npm run build`, which
// clears build/ (rowel-build.js); `bun run extension:build` at the root runs
// both.
//
// Safari takes the Firefox build nearly as it is — Manifest V2, a background
// page, the `browser` namespace — so its manifest is derived from
// dist/manifest_firefox.json rather than kept as a third copy to drift:
//
//   - the background page is persistent: Safari does not wake a suspended
//     background page for a native message, and the port has to keep polling
//     for lock signals (safari-native.js);
//   - safari-native.js is loaded right after the polyfill, before anything
//     that connects;
//   - the Gecko settings go (add-on id, data collection), as does any
//     manifest `key` (Chromium's pinned id means nothing to Safari), and the
//     blocking webRequest permission Safari does not support (upstream already
//     skips HTTP auth under Safari);
//   - the extension's icons are the PNG ones from the Chromium manifest.

import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const source = join(root, 'keepassxc-browser');
const out = join(root, 'build', 'safari');
const SHIM = 'background/safari-native.js';
const POLYFILL = 'common/browser-polyfill.min.js';

const readJson = (path) => JSON.parse(readFileSync(path, 'utf8'));

const firefox = readJson(join(root, 'dist', 'manifest_firefox.json'));
const chromium = readJson(join(root, 'dist', 'manifest_chromium.json'));
// The version is the source manifest's, as build.js sets it for the others.
const { version } = readJson(join(source, 'manifest.json'));

const manifest = structuredClone(firefox);
manifest.version = version;
delete manifest.applications;
delete manifest.browser_specific_settings;
delete manifest.key;
manifest.icons = chromium.icons;
manifest.permissions = manifest.permissions.filter((p) => p !== 'webRequestBlocking');

const scripts = manifest.background.scripts.filter((s) => s !== SHIM);
const at = scripts.indexOf(POLYFILL);
if (at === -1) {
    throw new Error(`the Firefox manifest's background has no ${POLYFILL}`);
}
scripts.splice(at + 1, 0, SHIM);
manifest.background = { scripts, persistent: true };

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });
// Like rowel-build.js: not the tests Playwright copies in while it runs.
const skip = join(source, 'tests');
cpSync(source, out, { recursive: true, filter: (src) => src !== skip });
cpSync(join(root, 'safari', 'safari-native.js'), join(out, SHIM));
writeFileSync(join(out, 'manifest.json'), JSON.stringify(manifest, null, 4) + '\n');

for (const script of scripts) {
    if (!existsSync(join(out, script))) {
        throw new Error(`the Safari manifest names ${script}, which is not in the build`);
    }
}
console.log(`Rowel: Safari extension ${version} in ${out}`);
