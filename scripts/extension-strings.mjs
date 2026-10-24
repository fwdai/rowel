// Rebrands the browser extension's user-facing strings
// (extension/rowel/_locales/*/messages.json) from KeePassXC to
// Rowel. Run it after every `git subtree pull` of upstream: taking upstream's
// locale files wholesale and re-running this resolves their conflicts.
//
// Every locale gets the product names swapped. English, the only locale kept
// correct by hand, also says "vault" for "database" and has the sentences that
// read wrong after a plain swap rewritten (OVERRIDES). Only "message" values
// are touched; translator notes and the files' formatting are left as they are,
// so the diff against upstream stays one line per changed string.
//
// `--check` changes nothing and exits 1 if a file is not rebranded (CI).
import { readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const LOCALES = 'extension/rowel/_locales'
const RELEASES = 'github.com/fwdai/rowel/releases'

// Upstream's name for the extension, as the translations spell it.
const EXTENSION_NAME =
  /KeePassXC[-–‑]?(?:Browser|browser|Navigator|Naviagator|böngésző|브라우저|ဘရောက်ဇာ|Tarayıcı|דפדפן־)/g

const OVERRIDES = {
  extensionDescription: 'Fill logins, one-time codes and passkeys from your Rowel vault.',
  connectButtonTitle: 'Connect the extension to the vault open in Rowel.',
  reconnectButtonTitle: 'Reconnect the extension to the vault open in Rowel.',
  removeConnectedDatabaseButtonTitle: 'Remove this connected vault from the extension.',
  errorMessageTimeout:
    'Cannot connect to Rowel. Check that Rowel is running and the browser extension is turned on in its Settings.',
  popupDownloadNewVersion: `Please download the latest version from ${RELEASES}`,
  popupTroubleshootingText: 'Cannot connect to Rowel? The troubleshooting guide may help.',
  popupNotConfigured:
    'The extension is not connected yet. Click the connect button to pair it with Rowel.',
  popupNeedReconfigure: 'The extension has been disconnected from Rowel.',
  popupConfiguredNotAssociated:
    'The extension has not yet connected to Rowel but has been configured using the identifier: $1',
  popupConfiguredAndAssociated:
    'The extension is connected to Rowel and has been configured using the following identifier: $1',
  optionsVersionInfoText: 'The extension needs the Rowel app to retrieve credentials.',
  // Rowel answers as the KeePassXC version whose protocol it speaks.
  optionsKeePassXCVersionRequired: 'Requires a Rowel app that speaks KeePassXC protocol $1 or newer.',
  optionsMinimumKeePassXCVersionRequired:
    'The extension requires a Rowel app that speaks KeePassXC protocol $1 or newer. Earlier versions can cause unwanted behavior.',
  optionsAboutKeePassXCVersion: 'KeePassXC protocol version reported by Rowel: $1',
  optionsAboutExtensionVersion: 'Extension version: $1',
  optionsConnectedDatabasesText: 'Vaults connected to this extension.',
  gettingStartedWelcomeText:
    'Welcome to the Rowel browser extension. It fills logins and passkeys from your Rowel vault.'
}

// Keys upstream does not have, [message, translator note]. They are appended
// to English when missing, so taking upstream's file wholesale and re-running
// this restores them; the other locales fall back to English for a key they
// lack, as Chrome and Firefox do.
const ROWEL_MESSAGES = {
  agentModeTitle: ['Agent mode', 'Rowel: label of the agent mode switch in the popup.'],
  agentModeDescription: [
    'For AI browsers. Rowel asks you before each fill and hides its buttons from the page.',
    'Rowel: helper text under the agent mode switch.'
  ],
  agentModeOn: ['Agent mode is on', 'Rowel: title of the popup screen shown while agent mode is on.'],
  agentModeOnDescription: [
    'Rowel asks you before each fill.',
    'Rowel: text of the popup screen shown while agent mode is on.'
  ],
  agentModeTurnOff: ['Turn off', 'Rowel: button that turns agent mode off.'],
  agentModeGrantDeniedNotification: [
    'Rowel did not allow this fill',
    'Rowel: notification when a fill was denied in agent mode.'
  ],
  agentModeGrantPendingNotification: [
    'Rowel is busy with another request. Try again in a moment.',
    "Rowel: notification when another request holds Rowel's consent dialog."
  ],
  agentModeAgentsDisabledNotification: [
    'Turn on Agents in Rowel › Settings › Browser & agents',
    'Rowel: notification when agents are turned off in the app.'
  ],
  agentModeRateLimitedNotification: [
    'Too many requests; wait a minute.',
    'Rowel: notification when the app rate-limits fill requests.'
  ],
  agentModeExposedNotification: [
    'Rowel ended the grant: the page showed the password',
    'Rowel: notification when a filled secret showed in the page and the grant was ended.'
  ],
  agentModeGrantEndedNotification: [
    'The agent is done: Rowel signed you out of the site in this browser',
    "Rowel: notification when an agent's grant ended and the site's cookies were deleted."
  ],
  errorMessageGrantDenied: ['Grant denied', 'Rowel: error code 40.'],
  errorMessageGrantPending: ['Grant pending approval', 'Rowel: error code 41.'],
  errorMessageGrantExpired: ['Grant expired', 'Rowel: error code 42.'],
  errorMessageNotPermittedForKind: ['Not permitted for this kind of client', 'Rowel: error code 43.'],
  errorMessagePolicyRequiresApproval: ['This fill needs your approval in Rowel', 'Rowel: error code 44.'],
  errorMessageRateLimited: ['Too many requests', 'Rowel: error code 45.'],
  errorMessageAgentsDisabled: ["Agents are turned off in Rowel's settings", 'Rowel: error code 46.']
}

const rename = text =>
  text
    .replace(EXTENSION_NAME, 'Rowel')
    .replace(/KeePassXC/g, 'Rowel')
    .replace(/keepassxc\.org/g, RELEASES)

const english = (key, text) =>
  OVERRIDES[key] ??
  rename(text).replace(/\b([Dd])atabase(s?)\b/g, (_, d, s) => `${d === 'D' ? 'V' : 'v'}ault${s}`)

// messages.json is one `"key": {` line per message with its `"message": "…"`
// on a line of its own; rewrite those lines in place.
function rebrand(source, locale) {
  let key
  return source
    .split('\n')
    .map(line => {
      const entry = line.match(/^ {4}"([^"]+)": \{/)
      if (entry) key = entry[1]
      const message = line.match(/^(\s*"message": )(".*")(,?)$/)
      if (!message) return line
      const text = JSON.parse(message[2])
      const next = locale === 'en' ? english(key, text) : rename(text)
      return next === text ? line : `${message[1]}${JSON.stringify(next)}${message[3]}`
    })
    .join('\n')
}

// Appends the ROWEL_MESSAGES the file lacks, in the file's own formatting.
function withRowelMessages(source) {
  const missing = Object.entries(ROWEL_MESSAGES).filter(([key]) => !source.includes(`\n    "${key}": {`))
  if (missing.length === 0) return source
  const entries = missing.map(
    ([key, [message, description]]) =>
      `    ${JSON.stringify(key)}: {\n        "message": ${JSON.stringify(message)},\n        "description": ${JSON.stringify(description)}\n    }`
  )
  return source.replace(/\n}\n?$/, `,\n${entries.join(',\n')}\n}\n`)
}

const check = process.argv.includes('--check')
const stale = []
for (const locale of readdirSync(LOCALES)) {
  const file = join(LOCALES, locale, 'messages.json')
  const source = readFileSync(file, 'utf8')
  const next = rebrand(locale === 'en' ? withRowelMessages(source) : source, locale)
  if (next === source) continue
  stale.push(file)
  if (!check) writeFileSync(file, next)
}

if (check && stale.length) {
  console.error(`Not rebranded (run \`bun run extension:strings\`):\n  ${stale.join('\n  ')}`)
  process.exit(1)
}
console.log(check ? 'Extension strings are rebranded.' : `Rebranded ${stale.length} locale file(s).`)
