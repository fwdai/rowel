// Fails when the macOS entitlements request something only a provisioning
// profile can grant, and the bundle has no profile that grants it.
//
// Why this exists: 1.0.0-alpha.1 and 1.0.0-alpha.2 both shipped a
// `keychain-access-groups` entitlement with no `embedded.provisionprofile`
// alongside it. That entitlement is profile-gated, so AMFI refused to spawn the
// process at all — SIGKILL before `main`, no crash report. Every other signal
// was green: valid signature, successful notarization, stapled ticket, `spctl`
// accepting the bundle. Finder said only "The application "Rowel" can't be
// opened." Nothing in the build caught it because nothing ever launched the app.
//
// The check is static (no `plutil`, no `security`, no Mach-O parsing) so it runs
// on any runner, and it is deliberately fail-closed: anything it cannot parse or
// verify is an error, never a pass.
//
// Usage: bun scripts/check-macos-entitlements.mjs

import { readFileSync, existsSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const entitlementsPath = join(root, 'src-tauri', 'Entitlements.plist')
const profilePath = join(root, 'src-tauri', 'embedded.provisionprofile')

const fail = (msg) => {
  console.error(`check-macos-entitlements: ${msg}`)
  process.exit(1)
}

// Entitlement keys macOS will only honour when an embedded provisioning profile
// grants them. Requesting any of these without a profile is not a degraded
// feature — it is an app that cannot start.
const PROFILE_GATED = [
  'keychain-access-groups',
  'com.apple.application-identifier',
  'com.apple.developer.team-identifier',
  'com.apple.security.application-groups',
]
const isProfileGated = (key) =>
  PROFILE_GATED.includes(key) || key.startsWith('com.apple.developer.')

// The one exception, and only for application groups: a macOS-style group id
// (`<team id>.<name>`) is granted by the signature's team id alone and needs no
// profile (Apple DTS, developer forums thread 721701). The Developer ID profile
// does not list application-groups at all, and the app and its Safari extension
// share `UFBL3F444A.app.rowel.desktop` without it. An iOS-style `group.` id is
// the opposite: macOS 15+ checks it against the profile, and without one
// granting it the process is SIGKILLed like any other gated entitlement.
const TEAM_ID = 'UFBL3F444A'
const APP_GROUPS = 'com.apple.security.application-groups'
const isTeamGroup = (value) => typeof value === 'string' && value.startsWith(`${TEAM_ID}.`)

// --- the smallest plist reader that covers both files ---------------------
// Handles the subset Apple emits: dict, array, string, true/false, date,
// integer. Returns plain JS values. Throws on anything unexpected so a format
// change surfaces as a failure rather than a silent empty result.
function parsePlist(xml) {
  let i = xml.indexOf('<plist')
  if (i === -1) throw new Error('no <plist> element')
  i = xml.indexOf('>', i) + 1

  const skipWhitespaceAndComments = () => {
    for (;;) {
      while (i < xml.length && /\s/.test(xml[i])) i++
      if (xml.startsWith('<!--', i)) {
        const end = xml.indexOf('-->', i)
        if (end === -1) throw new Error('unterminated comment')
        i = end + 3
        continue
      }
      return
    }
  }

  const readTag = () => {
    skipWhitespaceAndComments()
    if (xml[i] !== '<') throw new Error(`expected a tag at offset ${i}`)
    const end = xml.indexOf('>', i)
    if (end === -1) throw new Error('unterminated tag')
    const raw = xml.slice(i + 1, end)
    i = end + 1
    return { name: raw.replace(/\/$/, '').trim(), selfClosing: raw.endsWith('/') }
  }

  const readTextUntilClose = (name) => {
    const close = `</${name}>`
    const end = xml.indexOf(close, i)
    if (end === -1) throw new Error(`unterminated <${name}>`)
    const text = xml.slice(i, end)
    i = end + close.length
    return text
      .replace(/&lt;/g, '<')
      .replace(/&gt;/g, '>')
      .replace(/&quot;/g, '"')
      .replace(/&apos;/g, "'")
      .replace(/&amp;/g, '&')
  }

  const readValue = () => {
    const tag = readTag()
    switch (tag.name) {
      case 'dict': {
        const out = {}
        if (tag.selfClosing) return out
        for (;;) {
          skipWhitespaceAndComments()
          if (xml.startsWith('</dict>', i)) {
            i += '</dict>'.length
            return out
          }
          const keyTag = readTag()
          if (keyTag.name !== 'key') throw new Error(`expected <key>, got <${keyTag.name}>`)
          const key = readTextUntilClose('key')
          out[key] = readValue()
        }
      }
      case 'array': {
        const out = []
        if (tag.selfClosing) return out
        for (;;) {
          skipWhitespaceAndComments()
          if (xml.startsWith('</array>', i)) {
            i += '</array>'.length
            return out
          }
          out.push(readValue())
        }
      }
      case 'true':
        return true
      case 'false':
        return false
      case 'string':
        return tag.selfClosing ? '' : readTextUntilClose('string')
      case 'date':
        return new Date(readTextUntilClose('date'))
      case 'integer':
        return Number(readTextUntilClose('integer'))
      case 'real':
        return Number(readTextUntilClose('real'))
      case 'data':
        return readTextUntilClose('data')
      default:
        throw new Error(`unsupported plist element <${tag.name}>`)
    }
  }

  return readValue()
}

// A .provisionprofile is a CMS signature wrapping an XML plist. We only need to
// read it, not verify it — codesign does the verifying — so pull the payload out
// of the DER rather than shelling out to `security cms`.
function parseProfile(buf) {
  const text = buf.toString('latin1')
  const start = text.indexOf('<?xml')
  const end = text.indexOf('</plist>')
  if (start === -1 || end === -1) throw new Error('no plist payload found in profile')
  return parsePlist(text.slice(start, end + '</plist>'.length))
}

// `S8EX82NJP6.*` in a profile covers `S8EX82NJP6.anything`.
const granted = (requested, grantedList) =>
  grantedList.some((g) =>
    g.endsWith('*') ? requested.startsWith(g.slice(0, -1)) : g === requested,
  )

const readEntitlements = (path, label) => {
  if (!existsSync(path)) fail(`missing ${path}`)
  try {
    return parsePlist(readFileSync(path, 'utf8'))
  } catch (e) {
    fail(`could not parse ${label}: ${e.message}`)
  }
}

// What of `entitlements` a profile has to back: every profile-gated key, with
// application-groups narrowed to the groups the team id does not grant (and
// dropped when that leaves none). Team-id groups are reported, not verified.
const gatedRequests = (entitlements, label) => {
  const gated = {}
  for (const [key, value] of Object.entries(entitlements)) {
    if (!isProfileGated(key)) continue
    if (key !== APP_GROUPS) {
      gated[key] = value
      continue
    }
    if (!Array.isArray(value)) {
      fail(`${label}: "${APP_GROUPS}" is not a list of groups`)
    }
    const byTeam = value.filter(isTeamGroup)
    const others = value.filter((group) => !isTeamGroup(group))
    if (byTeam.length > 0) {
      console.log(
        `check-macos-entitlements: ${label}: ${byTeam.join(', ')} under "${APP_GROUPS}" ` +
          `is a macOS-style group (team id ${TEAM_ID}); the signature grants it, no profile needed.`,
      )
    }
    for (const group of others) {
      if (typeof group === 'string' && group.startsWith('group.')) {
        console.error(
          `check-macos-entitlements: ${label}: "${group}" is an iOS-style group id. On macOS 15+ ` +
            `it is profile-gated; use the team-id form, ${TEAM_ID}.<name>, instead.`,
        )
      }
    }
    if (others.length > 0) gated[key] = others
  }
  return gated
}

// --- run ------------------------------------------------------------------

const entitlements = readEntitlements(entitlementsPath, 'Entitlements.plist')
const gatedValues = gatedRequests(entitlements, 'app')
const requestedGated = Object.keys(gatedValues)

if (requestedGated.length === 0) {
  console.log(
    'check-macos-entitlements: no profile-gated entitlements requested; nothing to verify.',
  )
  process.exit(0)
}

if (!existsSync(profilePath)) {
  fail(
    `Entitlements.plist requests profile-gated entitlements (${requestedGated.join(', ')}) ` +
      `but src-tauri/embedded.provisionprofile does not exist.\n` +
      `  Without it macOS refuses to launch the app at all (SIGKILL before main), even ` +
      `though signing and notarization succeed.\n` +
      `  Add the Developer ID provisioning profile, or empty out Entitlements.plist. ` +
      `See docs/releasing.md.`,
  )
}

let profile
try {
  profile = parseProfile(readFileSync(profilePath))
} catch (e) {
  fail(`could not parse embedded.provisionprofile: ${e.message}`)
}

const problems = []

const platforms = profile.Platform ?? []
if (!platforms.includes('OSX')) {
  problems.push(`profile is for ${platforms.join('/') || 'an unknown platform'}, not macOS (OSX)`)
}

// The team-id groups passed above on the strength of the signature's team id:
// that only holds if the app is signed by the team this script assumes.
if (!(profile.TeamIdentifier ?? []).includes(TEAM_ID)) {
  problems.push(
    `profile is for team ${(profile.TeamIdentifier ?? []).join(', ') || '(none)'}, not ${TEAM_ID}`,
  )
}

// Developer ID profiles are the only kind that work for direct distribution;
// they are the ones marked as provisioning every device.
if (profile.ProvisionsAllDevices !== true) {
  problems.push(
    'profile is not a Developer ID profile (ProvisionsAllDevices is not true); ' +
      'a Development or App Store profile will not work for direct distribution',
  )
}

if (profile.ExpirationDate instanceof Date) {
  if (Number.isNaN(profile.ExpirationDate.getTime())) {
    problems.push('profile has an unreadable ExpirationDate')
  } else if (profile.ExpirationDate.getTime() < Date.now()) {
    problems.push(`profile expired on ${profile.ExpirationDate.toISOString().slice(0, 10)}`)
  }
} else {
  problems.push('profile has no ExpirationDate')
}

const profileEntitlements = profile.Entitlements ?? {}
for (const key of requestedGated) {
  if (!(key in profileEntitlements)) {
    problems.push(`profile does not grant "${key}"`)
    continue
  }
  const requested = gatedValues[key]
  const grantedValue = profileEntitlements[key]
  const describe = (v) => (Array.isArray(v) ? v.join(', ') : String(v))

  // Group-valued (keychain-access-groups, application-groups): the profile must
  // cover every group asked for.
  if (Array.isArray(requested)) {
    if (!Array.isArray(grantedValue)) {
      problems.push(`profile grants "${key}" but not as a list of groups`)
      continue
    }
    for (const value of requested) {
      if (typeof value !== 'string') {
        problems.push(`cannot verify a non-string value under "${key}"`)
      } else if (!granted(value, grantedValue)) {
        problems.push(
          `profile does not grant "${value}" under "${key}" (it grants: ${describe(grantedValue)})`,
        )
      }
    }
    continue
  }

  // Scalar-valued (application-identifier, team-identifier). Comparing these
  // matters as much as the groups: a profile issued for a different App ID
  // grants the key but not the value, and the app is refused the same way.
  if (typeof requested === 'string') {
    const grantedList = Array.isArray(grantedValue) ? grantedValue : [grantedValue]
    if (!grantedList.every((g) => typeof g === 'string')) {
      problems.push(`profile grants "${key}" as a value this check cannot compare`)
    } else if (!granted(requested, grantedList)) {
      problems.push(
        `profile does not grant "${requested}" under "${key}" (it grants: ${describe(grantedValue)})`,
      )
    }
    continue
  }

  if (typeof requested === 'boolean') {
    if (grantedValue !== requested) {
      problems.push(
        `entitlements ask for "${key}" = ${requested}, profile grants ${describe(grantedValue)}`,
      )
    }
    continue
  }

  // Anything else is unverifiable, and unverifiable means fail — the whole point
  // of this check is that an entitlement the profile does not back is fatal.
  problems.push(
    `cannot verify "${key}": unsupported entitlement value type "${typeof requested}"`,
  )
}

if (problems.length > 0) {
  fail(
    `src-tauri/embedded.provisionprofile does not authorize the requested entitlements:\n` +
      problems.map((p) => `  - ${p}`).join('\n'),
  )
}

console.log(
  `check-macos-entitlements: profile "${profile.Name ?? '(unnamed)'}" grants ` +
    `${requestedGated.join(', ')}; expires ${profile.ExpirationDate.toISOString().slice(0, 10)}.`,
)
