# Developing Rowel

Rowel is a [Tauri 2](https://v2.tauri.app) app: a Rust core (`src-tauri/`, with shared logic in `src-tauri/crates/core`) and a React + TypeScript + Vite frontend (`src/`). All cryptography lives in Rust, and the webview never sees the master key. Beside the core, `src-tauri/crates/broker` (`rowel-broker`) holds the browser extension host without the app around it — the KeePassXC-Browser protocol, the `Host` trait, the lockbox-side functions, and the grants, policy and audit chain that govern what agents may use — so the app, a headless broker and a Rowel Server link the same code; `src-tauri/crates/serve` (`rowel-serve`) is that headless broker — the `Host` over a device-held lockbox unlocked from a terminal or a key file, with its config, approvals queue, control socket and WebSocket listener (see [docs/serve.md](./serve.md)); and `src-tauri/crates/cli` (`rowel-cli`) is the command line tool that speaks to the running app as an agent client — the MCP server coding agents connect through — and as the sidecar that resolves a secret into one command's environment and redacts it from the output, and that runs the headless broker as `rowel serve` (see [docs/cli.md](./cli.md) for the reference and [docs/agents.md](./agents.md) for the user guide).

The Rowel Server, the broker in server mode, is not in this repository: it is a separate product that depends on this repository's `rowel-core`, `rowel-broker` and `rowel-serve` at a pinned revision. What stays here are its clients — the app's org lockbox support (`src-tauri/src/org`, tested against the in-process `org/fake_server.rs`) and the CLI's workload proof (`crates/cli/src/workload.rs`) — and the contract they speak, [docs/server-api.md](./server-api.md). A change to those three crates' public API is a change to the server's too.

The command line tool ships only with the packaged app, as a Tauri sidecar: the release workflow builds `rowel-cli` for the target, stages it as `src-tauri/binaries/rowel-cli-<target triple>` (gitignored) and bundles it through `src-tauri/tauri.release.conf.json`, which only the release passes to `--config`. A dev build never needs it. To try it locally, `cd src-tauri && cargo build -p rowel-cli` puts `target/debug/rowel-cli` beside the dev binary, which is where Settings › General › Command line tool looks for it; *Install* there puts it on your PATH as `rowel` (a link on macOS and Linux, a copy on Windows; see [cli.md](cli.md#installing)).

## Vocabulary

- **Vault** — everything Rowel keeps on a device: one lock, one master
  password, "Lock vault", Vault Health.
- **Lockbox** — a compartment inside the vault (Personal, Family, Work, a
  client). Each is its own SQLCipher file and syncs to Google Drive as its own
  pack; its sync identity is `vault_id` in the code (`store::identity`).
- **Item** (or secret) — what goes in a lockbox: a login, card, note, identity
  or document, SSH key, API key or env file. The code calls it an `Entry`.
- The code still calls a lockbox a **workspace**: `src-tauri/src/workspace.rs`,
  `src/store/workspace.ts`, `WorkspacePicker`, the `workspace_*` commands, the
  `workspaces:*` events, the `workspaces/` directory and `workspaces.json`. Docs
  say "lockbox" for the idea and keep the code name for a symbol or a path.

## Prerequisites

- [Bun](https://bun.sh) 1.3.3 or newer (what CI pins) — the package manager and
  script runner for this repo; `bun.lock` is the committed lockfile
- [Node.js](https://nodejs.org) 24.22.1 (what `.nvmrc` pins; CI reads it from
  there) — the E2E suite needs it, since WebdriverIO runs under Node, and so
  does the browser extension's tooling
- [Rust](https://rustup.rs) (stable toolchain)
- Platform build dependencies for Tauri — see the
  [Tauri prerequisites guide](https://v2.tauri.app/start/prerequisites/)
  (on Linux: `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libgtk-3-dev`)

## Install, run and test

```bash
git clone git@github.com:fwdai/rowel.git
cd rowel
bun install

bun run tauri:dev     # run the app in development
bun run tauri:build   # produce a signed, packaged build for the current OS

bun run tauri:dev:fresh   # same, against a throwaway data dir (first-run flows)

bun run build         # build the frontend only (tsc + vite)
bun run test          # frontend unit tests (Vitest)
cd src-tauri && cargo test   # backend tests
```

Run the scripts with `bun run <name>`, not `bun <name>`: bare `bun test` starts
Bun's own test runner instead of the Vitest suite this repo is written against.

`tauri:dev:fresh` points `ROWEL_DB_DIR` at a **new** temp directory each run, so
the app finds no vault there and starts at the setup/restore screen every time.
Your real dev vault is left untouched — `bun run tauri:dev` goes back to it.

Setting `ROWEL_DB_DIR` yourself overrides that and keeps the same directory
across runs, which is how you set a throwaway vault up once and then relaunch
into its unlock screen. Nothing here ever deletes a data directory; to start
that one over, remove it yourself. Debug builds only: a release build ignores
the variable and always uses the OS app-data directory.

## Building the iOS AutoFill extension

The iOS app embeds an AutoFill credential provider, the `rowel_autofill`
target in `src-tauri/gen/apple/project.yml`. Its Swift is in
`src-tauri/gen/apple/Sources/autofill`; the vault it reads is the
`rowel-autofill` crate (`src-tauri/crates/autofill`), which Xcode builds for
the SDK being targeted in a pre-build step, like the app's own Rust. Swift
reaches the crate through UniFFI bindings committed under
`Sources/autofill/generated`: after changing anything the crate exports, run
`src-tauri/crates/autofill/bindgen.sh` and commit what it writes.

## Configuration

### Google Drive sync (optional)

Release builds ship with a working Google OAuth client, so sync works out of
the box. This section only applies when building from source, where you supply
a client of your own. Desktop and iOS need one client each, because Google will
not let a Desktop client redirect to a mobile app.

#### Desktop

1. In the [Google Cloud Console](https://console.cloud.google.com) create an
   OAuth 2.0 Client ID of type **Desktop app**.
2. Enable the **Google Drive API** for the project.
3. The app requests the `https://www.googleapis.com/auth/drive.file` scope and
   listens on the loopback redirect URI `http://127.0.0.1:4567/auth/callback`.
4. Provide the credentials as environment/build variables when running or
   building:

   ```bash
   export GOOGLE_OAUTH_CLIENT_ID=your-desktop-client-id.apps.googleusercontent.com
   export GOOGLE_OAUTH_CLIENT_SECRET=your-desktop-client-secret   # optional
   ```

   They are read at runtime (`std::env::var`) and, if absent, fall back to the
   value baked in at compile time (`option_env!`). The build succeeds without
   them — sync simply reports "Google OAuth client not configured" until a
   client id is supplied.

#### iOS

An iOS client is a _public_ client: it has **no secret**, PKCE is mandatory, and
its redirect URI is its own client id reversed. So the one thing to configure is
that URL scheme, and the client id is derived from it.

1. In the same project create a second OAuth 2.0 Client ID, of type **iOS**,
   with the **iOS** bundle id from `src-tauri/tauri.ios.conf.json`
   (`app.rowel.mobile` — not the desktop one in `tauri.conf.json`).
2. Take the client id it gives you — `123456-abcdef.apps.googleusercontent.com`
   — and reverse it into a scheme: `com.googleusercontent.apps.123456-abcdef`.
   (Google shows this as the "iOS URL scheme" on the credential page.)
3. Add the scheme to `src-tauri/tauri.ios.conf.json` (there is no `plugins`
   block there until you do — a placeholder scheme is not a legal URL scheme
   and App Store Connect rejects the upload over it, so the file ships without
   one):

   ```json
   "plugins": {
     "deep-link": {
       "mobile": [{ "scheme": ["com.googleusercontent.apps.123456-abcdef"] }]
     }
   }
   ```

   This is the committed source of truth for iOS, and it is safe to commit —
   an iOS client id is public by design. The Tauri CLI registers the scheme as
   `CFBundleURLTypes` in the generated `Info.plist` during `tauri ios build`
   (via `tauri-plugin-deep-link`'s build script), and the app derives both the
   client id and the redirect URI `com.googleusercontent.apps.<id>:/oauth2redirect`
   from it at runtime. Re-run `bun run tauri ios init` if the Xcode project is
   out of date.

4. Add the **redirect URI** `com.googleusercontent.apps.123456-abcdef:/oauth2redirect`
   to the client in the console.

Release builds do not need the block committed: `bun run release:ios` and the
`Release iOS` workflow derive the scheme from `GOOGLE_OAUTH_IOS_CLIENT_ID` (in
`.env` / the repository secret) and pass it through `tauri ios build --config`,
which replaces a committed one. That is the only way in — exporting
`GOOGLE_OAUTH_CLIENT_ID` does nothing on iOS, because `tauri ios build` compiles
inside xcodebuild with a replaced environment that carries only the CLI's own
`TAURI_*` variables, so `option_env!` never sees it. `GOOGLE_OAUTH_CLIENT_SECRET`
is ignored on iOS and must never be shipped in a mobile binary.

### Auto-update signing

Release builds are signed for `tauri-plugin-updater`. The public key lives in
`src-tauri/tauri.conf.json`; the matching **private key is never committed** and
is provided to CI via the `TAURI_SIGNING_PRIVATE_KEY` (and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) secrets. Generate a keypair with
`bun run tauri signer generate -w ~/.rowel/updater.key`.

## Extending Rowel

- **New import format:** add an `Importer` adapter and a `Format` variant in `src-tauri/crates/core/src/import/`.
- **New item type:** add a kind under `src/kinds/`.
- **Browser extension:** `bun run extension:install`, then `bun run extension:build`. See [docs/browser-extension.md](./browser-extension.md).

See also [CONTRIBUTING.md](../CONTRIBUTING.md) and [SECURITY.md](../SECURITY.md).
