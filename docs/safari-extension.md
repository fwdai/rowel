# The Safari extension (macOS)

Rowel's browser extension (the KeePassXC-Browser fork in `extension/`,
[browser-extension.md](browser-extension.md)) runs in
Safari on macOS as well as in Chrome and Firefox. It ships inside Rowel.app,
as the app extension `Contents/PlugIns/Rowel Safari Extension.appex`; there is
nothing to download separately. The user turns it on in Safari › Settings ›
Extensions, once Rowel has been launched from `/Applications`.

## Architecture

Chrome and Firefox launch a native messaging host, the `rowel` binary in proxy
mode, which relays the extension's stdio to the running app's local socket
(`src-tauri/src/browser/proxy.rs`). Safari launches no host. Its extensions
talk to a native handler in an app extension instead, and that app extension
is sandboxed:

```
Safari extension JS                 Rowel Safari Extension.appex        Rowel.app
(background page)                   (sandboxed)                          (not sandboxed)

client.js ─ port.postMessage ─┐
                              │ sendNativeMessage
safari-native.js ─────────────┴──▶ SafariWebExtensionHandler ─┐
   ▲  replies + signals              HostConnection (static) ──┴─ browser.sock ─▶ browser::server
   └──────────────────────────────── { reply, after, signals }      (App Group container)
```

- **The sockets.** Chrome and Firefox reach the app through `browser.sock` in
  its data directory, as they always have (`browser::root_dir`, what the proxy
  resolves). A sandboxed process cannot reach that, so on macOS the app also
  listens on a second `browser.sock`, for Safari alone, in the App Group
  container the app and the extension share,
  `~/Library/Group Containers/UFBL3F444A.app.rowel.desktop/` (a debug build's
  is in its `dev/` subdirectory, as everywhere else; `browser::group_socket_dir`
  decides the path). Both feed the same accept handling, lock signals
  included (`browser::server::start`). The data-directory listener is bound
  first and decides whether the host is on at all, as before; the Safari one
  is bound best-effort after it, and a failure there — the path too long for a
  socket address, the container not creatable — is logged as "Safari
  extension unavailable: …" and affects nothing else. A debug build run with
  `ROWEL_DB_DIR` does not bind it. The app creates the container directory
  (`0700`) if it is missing, which is what an unsigned `tauri dev` build relies
  on: the system makes a group container only for a signed, entitled process
  that asks for it.
- **The group** is `UFBL3F444A.app.rowel.desktop`
  (`rowel_core::app::DESKTOP_APP_GROUP`), in the macOS form: team id, dot,
  name. macOS grants that form on the signature's team id alone, with no
  provisioning profile. The iOS form, `group.…`, is checked against the
  profile on macOS 15+ and our Developer ID profile does not grant it — a
  build asking for it would be killed at launch (see the comment in
  `src-tauri/Entitlements.plist`). `scripts/check-macos-entitlements.mjs`
  enforces this for the app and the extension.
- **The handler** (`src-tauri/gen/safari/Sources`) keeps one connection to the
  socket in static state, made lazily: Safari may create a handler per
  message, but the app keeps a key exchange per connection. Each message is
  written as one frame of the host's framing (a four-byte native-endian length,
  then JSON, 1 MiB at most — `browser/frame.rs`) and answered with the next
  frame that is not a signal. One request is on the wire at a time. The
  handler answers:
  - `not-running` when nothing listens on the socket (the app is not running,
    or the browser integration is off);
  - `disconnected` when the connection ended (EOF, a frame over the cap, no
    reply within 90 s), or when there is no connection and the request is not
    `change-public-keys` — a new connection has no keys, so the extension has
    to reconnect and exchange them first.

  The connection is dropped on either, and made again by the next
  `change-public-keys`.
- **Signals.** The app pushes `database-locked` and `database-unlocked` to
  every connection unsolicited (`browser::server::signal`). A reader thread in
  the handler takes them off the socket, numbers them, and queues them (the
  last eight) for the next reply to carry, with the number of the last signal
  sent before that reply so the extension hears them in the app's order. It
  also tries `SFSafariApplication.dispatchMessage` to push each one to the
  extension straight away. Apple documents that call for the containing app,
  not for the extension process, so it is a shortcut when it works and nothing
  depends on it.
- **The extension's side** (`extension/safari/safari-native.js`, loaded only
  by the Safari build, right after the polyfill) replaces
  `runtime.connectNative` with a port that behaves as `background/client.js`
  expects of Chrome's: `postMessage` sends a request through
  `sendNativeMessage`, replies and signals come back through `onMessage`
  (deduplicated by number), and `onDisconnect` fires on any error — the "not
  connected" state, from which upstream's automatic reconnect takes over, just
  as when a Chrome proxy exits. While connected it polls the handler every two
  seconds, because Safari never lets a handler speak first: that heartbeat is
  what brings lock signals in when nothing else is being asked, and what
  notices the app has quit. The Safari manifest is Manifest V2 with a
  persistent background page, since Safari does not wake a suspended
  background page for native messages; it is generated from the Firefox one
  (`extension/safari/build.mjs`).

## Building

`scripts/build-safari-extension.sh` does the whole thing, and `tauri build`
runs it on macOS as `build.beforeBundleCommand` (`src-tauri/tauri.macos.conf.json`,
merged over `tauri.conf.json` for macOS only):

1. `bun run build:safari` in `extension/` — the Safari flavour, unpacked in
   `extension/build/safari/` (no npm install needed; `bun run extension:build`
   at the root builds it too, with the other browsers, and checks it);
2. `xcodegen generate` in `src-tauri/gen/safari/` (from cache: only when
   `project.yml` changed; the generated project is committed, like the iOS
   one);
3. `xcodebuild`, unsigned, arm64 + x86_64 (the app is universal), `debug`
   configuration for `tauri build --debug` and `release` otherwise; a build
   phase copies the web extension into the bundle's `Resources`;
4. `codesign`: with `APPLE_SIGNING_IDENTITY` (the Developer ID identity the app
   is signed with) plus hardened runtime and a secure timestamp, importing
   `APPLE_CERTIFICATE` into a throwaway keychain if the identity is not in
   one (CI); ad hoc when no identity is set.

The result is `src-tauri/gen/safari/build/Rowel Safari Extension.appex`, which
Tauri copies to `Contents/PlugIns/` through `bundle.macOS.files`. Tauri does
not sign it — it re-signs only nested code it put in place itself, and signs
the outer app last — which is why the script must sign it first, fully: an
unsigned or ad-hoc nested bundle fails notarization.

Requirements: Xcode (the deployment target is macOS 12, the oldest current
Xcode builds for), bun, and `xcodegen` when `project.yml` changed
(`brew install xcodegen`; without it the committed project is built as is).

To change the extension target (sources, Info.plist keys, entitlements), edit
`src-tauri/gen/safari/project.yml`, run `xcodegen generate` in that directory
and commit the regenerated project with it. The entitlements file is written
from `project.yml`; do not edit it by hand.

## Developing

`tauri dev` does not bundle, so it has no extension; the app it runs does
listen at the group container's `dev/browser.sock` (unless `ROWEL_DB_DIR` is
set). To try the extension:

```sh
# A debug app bundle, extension included (ad hoc signed without an identity).
TAURI_SIGNING_PRIVATE_KEY="$(cat /tmp/test.key)" bun run tauri build --debug --bundles app
open src-tauri/target/debug/bundle/macos/Rowel.app
```

(the key as in [releasing.md](releasing.md), "Local signed build".) Then in
Safari:

1. Settings › Advanced › "Show features for web developers".
2. Develop › Developer Settings… › **Allow unsigned extensions** (asks for your
   password; Safari turns it off again when it quits). Needed for an ad-hoc
   signed extension, not for a release build.
3. Settings › Extensions › enable **Rowel**, and grant it access to websites.
4. Turn on the browser integration in Rowel's Settings › Browser extension.

If the extension does not appear, make sure only one Rowel.app is registered
(`pluginkit -mAvvv -p com.apple.Safari.web-extension` lists what Safari sees)
and that Rowel was launched at least once. The background page's console is
under Develop › Web Extension Background Content › Rowel; the handler logs to
the unified log:

```sh
log stream --predicate 'subsystem == "app.rowel.desktop.safari"' --level info
```

The Swift relay can be exercised without Safari: a debug build honours
`ROWEL_DB_DIR` for the socket, as the app does, so a small driver calling
`SafariWebExtensionHandler.answer(_:)` can talk to a test app — this is how it
was checked against a fake app (framing, signal ordering, the cap, EOF,
reconnection).

## Verifying a release build

After `tauri build` (or on the artifact CI produced):

```sh
APP="src-tauri/target/universal-apple-darwin/release/bundle/macos/Rowel.app"
APPEX="$APP/Contents/PlugIns/Rowel Safari Extension.appex"

# The whole bundle, nested code included, verifies:
codesign --verify --strict --deep --verbose=2 "$APP"

# Both signed by the team, hardened runtime, with a timestamp:
codesign -dvv "$APP"   2>&1 | grep -E 'Authority|TeamIdentifier|flags|Timestamp'
codesign -dvv "$APPEX" 2>&1 | grep -E 'Authority|TeamIdentifier|flags|Timestamp'

# The extension is sandboxed and in the group; the app is in the group too:
codesign -d --entitlements - --xml "$APPEX"
codesign -d --entitlements - --xml "$APP"

# Gatekeeper accepts the notarized app:
spctl --assess --type execute --verbose=4 "$APP"
```

The release workflow runs the first three checks after its launch smoke test
("Check the bundled Safari extension"). The launch smoke test itself says
nothing about the extension, which Safari starts, not the app: an extension
killed for an ungranted entitlement would only show as Safari never loading
it, which is why the entitlements check covers the extension's file too.

## Limits

- `sun_path` is 104 bytes on macOS and the group container path is long: a
  home directory under `/Users` with a short name of more than 29 characters
  (25 for a debug build) leaves no room for the Safari socket. The app logs
  "Safari extension unavailable" and Safari's extension shows as not
  connected; Chrome and Firefox are unaffected (see the boundary test in
  `browser/tests.rs`).
- Signals arrive within the heartbeat (two seconds) unless the push from the
  handler turns out to work.
- HTTP basic-auth filling is not available in Safari (upstream skips it; Safari
  has no blocking `webRequest`).
- Safari may end the extension process when idle; the connection goes with it,
  the next heartbeat reports the extension disconnected, and upstream
  reconnects.
