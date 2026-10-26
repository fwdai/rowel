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
//     unpacked build has the pinned extension ID;
//   - the version has one home, rowel/manifest.json: the templates in dist/
//     carry none, and every manifest written gets it from there.
//
// Output: build/chromium/, build/firefox/, build/rowel_<version>_<browser>.zip
//
// `node rowel-build.js --source <chromium|firefox>` (`npm run debug:…`)
// builds nothing: it writes that browser's manifest into rowel/, for loading
// the source folder unpacked, keeping the version it had.

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

// A browser's manifest: its template with the version in it, where the
// template's upstream had it, after the name. Chromium also gets
// `version_name`, which it shows in place of the version.
const manifestFor = (browser) => {
    const { manifest_version, name, ...rest } = readJson(BROWSERS[browser]);
    const named = browser === 'chromium' ? { version_name: version } : {};
    return { manifest_version, name, version, ...named, ...rest };
};

const build = () => {
    fs.rmSync(OUT, { recursive: true, force: true });
    for (const browser of Object.keys(BROWSERS)) {
        console.log(`Rowel: creating extension package for ${browser}`);
        const dir = path.join(OUT, browser);
        fs.cpSync(SOURCE, dir, { recursive: true, filter: (src) => !SKIP.has(src) });

        const manifest = manifestFor(browser);
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
};

const source = process.argv.indexOf('--source');
if (source === -1) {
    build();
} else {
    const browser = process.argv[source + 1];
    if (!Object.hasOwn(BROWSERS, browser)) {
        throw new Error(`--source takes ${Object.keys(BROWSERS).join(' or ')}`);
    }
    writeJson(path.join(SOURCE, 'manifest.json'), manifestFor(browser));
}
