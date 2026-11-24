// Prints the `--config` patch that gives a phone build its sign-ins: the
// deep-link schemes Google's and Dropbox's OAuth redirects come back on, which
// is also where the app reads the client id and the app key from
// (src-tauri/src/sync/providers). Prints `{}` when neither is set, so the build
// goes ahead without them and keeps whatever its config file registers.
//
// The values come from the environment, or from `.env`, which bun loads by
// itself when this runs from the repo root:
//
//   GOOGLE_OAUTH_ANDROID_CLIENT_ID   the Google client of type Android
//   GOOGLE_OAUTH_IOS_CLIENT_ID       the Google client of type iOS
//   DROPBOX_APP_KEY                  the Dropbox app key, same on every platform
//
// A scheme is registered verbatim (an Android intent filter, an iOS URL type),
// so a value that is not what it claims to be is refused here rather than
// built into an app whose redirect cannot come back. The rules are those of
// scripts/release-ios.sh, which builds its own patch.
//
// Usage: tauri android dev --config "$(bun scripts/deep-link-config.mjs android)"

const GOOGLE_CLIENT_ID_ENV = {
  android: 'GOOGLE_OAUTH_ANDROID_CLIENT_ID',
  ios: 'GOOGLE_OAUTH_IOS_CLIENT_ID',
}
const DROPBOX_APP_KEY_ENV = 'DROPBOX_APP_KEY'

const GOOGLE_CLIENT_ID_SUFFIX = '.apps.googleusercontent.com'
const GOOGLE_CLIENT_ID = /^[A-Za-z0-9.+-]+\.apps\.googleusercontent\.com$/
const DROPBOX_APP_KEY = /^[A-Za-z0-9]+$/

const fail = (msg) => {
  console.error(`deep-link-config: ${msg}`)
  process.exit(1)
}

const platform = process.argv[2]
const googleEnv = GOOGLE_CLIENT_ID_ENV[platform]
if (!googleEnv) {
  fail(`usage: bun scripts/deep-link-config.mjs <${Object.keys(GOOGLE_CLIENT_ID_ENV).join('|')}>`)
}

// Registration order: Google's, then Dropbox's.
const schemes = []

const clientId = process.env[googleEnv]
if (clientId) {
  if (!GOOGLE_CLIENT_ID.test(clientId)) {
    fail(`${googleEnv} is not a Google OAuth client id (<id>${GOOGLE_CLIENT_ID_SUFFIX})`)
  }
  // The client id reversed, which is the scheme Google redirects to.
  schemes.push(`com.googleusercontent.apps.${clientId.slice(0, -GOOGLE_CLIENT_ID_SUFFIX.length)}`)
} else {
  console.error(`deep-link-config: ${googleEnv} is not set, building without Google Drive sign-in`)
}

const appKey = process.env[DROPBOX_APP_KEY_ENV]
if (appKey) {
  if (!DROPBOX_APP_KEY.test(appKey)) {
    fail(`${DROPBOX_APP_KEY_ENV} is not a Dropbox app key (letters and digits only)`)
  }
  schemes.push(`db-${appKey}`)
} else {
  console.error(`deep-link-config: ${DROPBOX_APP_KEY_ENV} is not set, building without Dropbox sign-in`)
}

// One entry holding every scheme, with no `host`: a custom scheme needs none,
// and without one the Android intent filter matches the scheme alone, which
// both redirects need (Google's has no authority, Dropbox's is `2`).
const config = schemes.length ? { plugins: { 'deep-link': { mobile: [{ scheme: schemes }] } } } : {}

console.log(JSON.stringify(config))
