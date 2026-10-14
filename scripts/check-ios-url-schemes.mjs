// Fails when a deep-link scheme in the iOS config is not a legal URL scheme.
// The Tauri CLI ships these verbatim as CFBundleURLTypes, and App Store Connect
// rejects the upload over a malformed one — error 90158, "URL schemes need to
// begin with an alphabetic character, and be comprised of alphanumeric
// characters, the period, the hyphen or the plus sign only" (RFC 1738). That
// rejection arrives at the end of a full build *and* a full upload, on both the
// local script and the release workflow, so both run this first.
//
// The scheme that matters here is Google's reversed iOS client id (README,
// "Drive sync on iOS"). Two committed files can carry one: the deep-link block
// in src-tauri/tauri.ios.conf.json, and the generated (but committed) Xcode
// Info.plist, which the plugin's build script rewrites only when it runs — so
// a placeholder left there ships verbatim when it does not.
//
// Usage: bun scripts/check-ios-url-schemes.mjs

import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const configPath = join(root, 'src-tauri', 'tauri.ios.conf.json')
const plistPath = join(root, 'src-tauri', 'gen', 'apple', 'rowel_iOS', 'Info.plist')

const fail = (msg) => {
  console.error(msg)
  process.exit(1)
}

const read = (path) => {
  try {
    return readFileSync(path, 'utf8')
  } catch (e) {
    return fail(`check-ios-url-schemes: cannot read ${path}: ${e.message}`)
  }
}

// RFC 1738 as Apple enforces it: leading letter, then letters, digits, '.', '+', '-'.
const LEGAL = /^[A-Za-z][A-Za-z0-9.+-]*$/

// plugins > deep-link > mobile is a list of entries, each with a `scheme` list.
// Absent is fine: an iOS build with no deep link registers no URL types.
const entries = JSON.parse(read(configPath)).plugins?.['deep-link']?.mobile ?? []
const configured = entries.flatMap((entry) => entry.scheme ?? [])

// Every <string> inside a CFBundleURLSchemes array. The plist is XML the Tauri
// CLI writes with one element per line, so a line-based scan is enough.
const inPlist = [...read(plistPath).matchAll(/<key>CFBundleURLSchemes<\/key>\s*<array>([\s\S]*?)<\/array>/g)]
  .flatMap(([, body]) => [...body.matchAll(/<string>([^<]*)<\/string>/g)].map(([, s]) => s))

const bad = [
  ...configured.filter((s) => !LEGAL.test(s)).map((s) => `${s}  (src-tauri/tauri.ios.conf.json)`),
  ...inPlist.filter((s) => !LEGAL.test(s)).map((s) => `${s}  (src-tauri/gen/apple/rowel_iOS/Info.plist)`),
]
if (bad.length > 0) {
  console.error('check-ios-url-schemes: illegal URL scheme for the iOS build:')
  for (const s of bad) console.error(`  ${s}`)
  console.error(
    '\nApp Store Connect rejects the upload with error 90158. A scheme must start\n' +
      'with a letter and hold only letters, digits, ".", "+" or "-".\n' +
      'A scheme is the reversed iOS OAuth client id (README, "Drive sync on iOS") or\n' +
      'db-<Dropbox app key>; fix it in the file named, or remove it to ship without\n' +
      "that provider's sign-in on iOS.",
  )
  process.exit(1)
}

const schemes = [...new Set([...configured, ...inPlist])]

console.log(
  schemes.length === 0
    ? 'check-ios-url-schemes: no deep-link schemes configured'
    : `check-ios-url-schemes: ${schemes.length} scheme(s) OK`,
)
