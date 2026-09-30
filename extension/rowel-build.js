#!/usr/bin/env node
'use strict';

// Rowel's build, in place of upstream's build.js (kept untouched so upstream
// syncs never conflict on it). Differences from upstream:
//   - never pulls translations from Transifex;
//   - stages each browser's copy under build/<browser>/ instead of swapping
//     rowel/manifest.json in place, so the source tree is left as
//     it was and the staged folders can be loaded unpacked;
//   - zips with `zip` rather than `tar -a`, which only writes a zip with BSD tar;
//   - drops the manifest `key` from the Chromium store zip: the Chrome Web
//     Store refuses uploads that carry one. build/chromium/ keeps it, so an
//     unpacked build has the pinned extension ID.
//
// Output: build/chromium/, build/firefox/, build/rowel_<version>_<browser>.zip

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const SOURCE = 'rowel';
const OUT = 'build';
const BROWSERS = {
    chromium: 'dist/manifest_chromium.json',
    firefox: 'dist/manifest_firefox.json',
};
// Copied into rowel/ by the Playwright global setup while tests run.
const SKIP = new Set([ path.join(SOURCE, 'tests') ]);

const readJson = (file) => JSON.parse(fs.readFileSync(file, 'utf8'));
const writeJson = (file, data) => fs.writeFileSync(file, JSON.stringify(data, null, 4) + '\n');

const zip = (dir, file) => {
    fs.rmSync(file, { force: true });
    execFileSync('zip', [ '-q', '-r', '-X', path.resolve(file), '.' ], { cwd: dir, stdio: 'inherit' });
};

const version = readJson(path.join(SOURCE, 'manifest.json')).version;
fs.rmSync(OUT, { recursive: true, force: true });

for (const [ browser, manifestFile ] of Object.entries(BROWSERS)) {
    console.log(`Rowel: creating extension package for ${browser}`);
    const dir = path.join(OUT, browser);
    fs.cpSync(SOURCE, dir, { recursive: true, filter: (src) => !SKIP.has(src) });

    const manifest = readJson(manifestFile);
    manifest.version = version;
    if (Object.hasOwn(manifest, 'version_name')) {
        manifest.version_name = version;
    }
    writeJson(path.join(dir, 'manifest.json'), manifest);

    const zipFile = path.join(OUT, `rowel_${version}_${browser}.zip`);
    if (manifest.key) {
        const store = path.join(OUT, `.store-${browser}`);
        fs.cpSync(dir, store, { recursive: true });
        delete manifest.key;
        writeJson(path.join(store, 'manifest.json'), manifest);
        zip(store, zipFile);
        fs.rmSync(store, { recursive: true, force: true });
    } else {
        zip(dir, zipFile);
    }
    console.log(`  ${dir}/ (unpacked), ${zipFile}`);
}
