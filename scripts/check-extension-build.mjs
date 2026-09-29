// Checks the browser extension's build output (extension/build, from
// `bun run extension:build`) against the IDs the desktop host allows in
// src-tauri/src/browser/manifest.rs: a mismatch there only shows up as a
// browser that silently cannot reach Rowel.
//
// The Chromium ID is derived from the manifest `key`, the way Chromium does:
// the first 32 hex digits of the key's SHA-256, spelled with a–p.
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

const BUILD = 'extension/build'
const HOST = 'app.rowel.browser'
const CHROMIUM_ID = 'dimghkhcdfaokfingegmgbnpnpcoeofj'
const FIREFOX_ID = 'browser@rowel.app'

const readJson = file => JSON.parse(readFileSync(file, 'utf8'))
const zippedManifest = zip => JSON.parse(execFileSync('unzip', ['-p', zip, 'manifest.json']))
const chromiumId = key =>
  [...createHash('sha256').update(Buffer.from(key, 'base64')).digest('hex').slice(0, 32)]
    .map(c => String.fromCharCode(97 + parseInt(c, 16)))
    .join('')

let failed = false
const check = (ok, message) => {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${message}`)
  failed ||= !ok
}

const chromium = readJson(join(BUILD, 'chromium', 'manifest.json'))
const firefox = readJson(join(BUILD, 'firefox', 'manifest.json'))
const store = zippedManifest(join(BUILD, `rowel_${chromium.version}_chromium.zip`))
const firefoxZip = zippedManifest(join(BUILD, `rowel_${firefox.version}_firefox.zip`))

check(
  chromium.key && chromiumId(chromium.key) === CHROMIUM_ID,
  `unpacked Chromium build pins ID ${CHROMIUM_ID}`
)
check(!store.key, 'Chromium store zip has no manifest key')
check(firefox.browser_specific_settings?.gecko?.id === FIREFOX_ID, `Firefox build has add-on ID ${FIREFOX_ID}`)
check(firefoxZip.browser_specific_settings?.gecko?.id === FIREFOX_ID, `Firefox zip has add-on ID ${FIREFOX_ID}`)
// The Safari flavour (extension/safari/build.mjs, docs/safari-extension.md):
// Safari ignores the host name, but the build should still be the one the
// others are, plus the native port shim loaded before anything connects.
const safari = readJson(join(BUILD, 'safari', 'manifest.json'))
const scripts = safari.background?.scripts ?? []
check(safari.version === firefox.version, `Safari build is version ${firefox.version}`)
check(!safari.key && !safari.browser_specific_settings, 'Safari build has no Chromium key or Gecko settings')
check(safari.background?.persistent === true, 'Safari build has a persistent background page')
check(
  scripts.indexOf('background/safari-native.js') === scripts.indexOf('common/browser-polyfill.min.js') + 1 &&
    scripts.indexOf('background/safari-native.js') < scripts.indexOf('background/client.js'),
  'Safari build loads the native port shim after the polyfill, before client.js'
)
for (const browser of ['chromium', 'firefox', 'safari']) {
  const client = readFileSync(join(BUILD, browser, 'background', 'client.js'), 'utf8')
  check(client.includes(`nativeHostName = '${HOST}'`), `${browser} build talks to native host ${HOST}`)
}

if (failed) process.exit(1)
