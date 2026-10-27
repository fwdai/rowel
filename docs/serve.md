# The headless broker: `rowel serve`

`rowel serve` is the Rowel app's agent broker without the app: the same
sealed channel, grant engine, policy and audit chain (`rowel-broker`), run
from the command line over a lockbox on a box with no desktop — a developer's
VM, a CI runner, a Linux server. Agents and sidecars connect to it the way
they connect to the app, over the data directory's socket or over a
WebSocket, and get exactly what they get from the app: a grant id and an
outcome, never a value. What the app's dialog does, a terminal does here:
standing rules in a config file serve agents without a person, and an ask
waits for `rowel approve` or times out. The crate is `src-tauri/crates/serve`
(`rowel-serve`); the CLI links it.

## Trust model, in two paragraphs

**Same-user approvals.** The app's dialog is a person at the screen; here it
is a person at a terminal on the same box, running as the same user. `rowel
approve` reaches the broker over an owner-only control socket beside the
broker's own (`serve.sock`, `0600`, in the `0700` data directory), so whoever
can answer an ask is already the user the broker runs as — who could read
the terminal they would otherwise type at (spec `10` Q9). A step-up ask
(cards, identities, anything a policy marks as money) has no biometrics to
fall back on: `rowel approve` asks for the master password and sends it over
that socket only; the broker derives it through the lockbox's own KDF
sidecar and compares the key material in constant time, exactly as the app
proves a password. A mismatch refuses the ask.

**What unattended means.** A standing rule with `approval: auto` serves the
agents it names while the broker is unlocked, with no one in the loop; that
is the point of a broker on a CI runner, and it is also the whole of what a
process on that box can get by speaking as the agent. The key file is the
vault's key: whoever reads it opens the lockbox. Keep it outside the data
directory, readable by the service user alone — under systemd, as a
credential the unit loads (below). Nothing is served from a sealed lockbox;
`SIGTERM`, `rowel admin lock` and the idle lock seal it, and every grant
goes with the seal.

**One broker per data directory.** The app and `rowel serve` take the same
exclusive lock, `broker.lock` in the data directory: the app when it binds
the extension's socket, the broker before it asks for a password or binds
anything, whatever it listens on. Whichever is second is refused — the
broker exits saying so, the app logs it and does not serve the extension —
so neither can replace the other's live socket on its way to failing, and no
two brokers write the same lockbox at once. The lock is
held until the process ends and released by the kernel however it ends, so a
crash needs no cleanup.

## Installing on a Linux box

The broker serves a *device-held* lockbox: a copy of the app's data
directory, or of one lockbox's files out of it. The lockbox's agent access
must be on, as in the app.

1. On the device, turn on **Agents can use items in this lockbox** for the
   lockbox agents will use (or do it on the box with `rowel admin agents
   on --lockbox <id>`), put the items they need in it, and quit the app.
2. Copy the data directory to the box — on macOS
   `~/Library/Application Support/app.rowel.desktop`, on Linux
   `~/.local/share/app.rowel.desktop` (a debug build's is the `dev`
   subdirectory). The primary lockbox *is* the directory; another lockbox is
   `workspaces/<id>` in it beside `workspaces.json`. The device's
   `install_id` file travels with it and is left alone: the broker writes its
   audit rows under an id of its own, minted on its first run into
   `serve_writer_id` (owner-only), so the device's chain and the box's never
   fork. Copying a box's data directory on to another box carries that file
   too; delete `serve_writer_id` from the copy so the new box mints its own.
   The vault is served as it was copied — the broker does not sync.
3. Install the command line tool ([below](#installing-the-command-line-tool)),
   reached as `rowel` from here on.
4. Unlock and serve:

   ```bash
   rowel serve --data-dir /srv/rowel          # asks for the master password
   ```

   The lines on stderr say which lockbox is open and where it listens.

## Installing the command line tool

`rowel serve` is the command line tool the app bundles. On a box without the
app, download its archive from the
[release](https://github.com/fwdai/rowel/releases) of the version you run:

| Archive | For |
| --- | --- |
| `rowel-<version>-x86_64-unknown-linux-gnu.tar.gz` | Linux on x86_64 with glibc 2.35 or later: Ubuntu 22.04, Debian 13, Fedora 36 and later |
| `rowel-<version>-aarch64-unknown-linux-gnu.tar.gz` | Linux on arm64, the same |
| `rowel-<version>-universal-apple-darwin.zip` | macOS on Apple silicon and Intel, signed and notarized |
| `rowel-<version>-x86_64-pc-windows-msvc.zip` | Windows on x64, not code-signed |

Each holds one file, `rowel` (`rowel.exe` on Windows). Check it against the
release's `SHA256SUMS`, then put it on the `PATH`:

```bash
VERSION=1.1.0 TARGET=x86_64-unknown-linux-gnu
BASE=https://github.com/fwdai/rowel/releases/download/v$VERSION
curl -fLO "$BASE/rowel-$VERSION-$TARGET.tar.gz"
curl -fLO "$BASE/SHA256SUMS"
grep " rowel-$VERSION-$TARGET.tar.gz\$" SHA256SUMS | sha256sum --check
tar -xzf "rowel-$VERSION-$TARGET.tar.gz"
install -m 0755 rowel ~/.local/bin/rowel     # or: sudo install -m 0755 rowel /usr/local/bin/
rowel --help
```

Every archive also has a build provenance attestation:
`gh attestation verify rowel-$VERSION-$TARGET.tar.gz --repo fwdai/rowel`.

- **macOS**: `shasum -a 256 --check` in place of `sha256sum --check`, and
  `unzip` in place of `tar`. A browser marks what it downloads as
  quarantined; the binary is notarized, so Gatekeeper lets it run after
  checking that online. On a box that cannot reach Apple, clear the mark
  instead: `xattr -d com.apple.quarantine rowel`.
- **Windows**: compare `(Get-FileHash rowel-<version>-x86_64-pc-windows-msvc.zip).Hash`
  with its line in `SHA256SUMS` (the case differs), unzip it, and put
  `rowel.exe` in a folder on your `PATH`. It is not code-signed, so
  SmartScreen may warn the first time.
- **From source**: `cargo build --release -p rowel-cli --manifest-path
  src-tauri/Cargo.toml` writes `src-tauri/target/release/rowel-cli`; install
  it as `rowel`.

## Unlock modes

- **Password** (default): asked for on the terminal without echo, or read
  as one line from stdin when that is not a terminal. After an idle lock the
  terminal is asked again.
- **`--key-file <path>`**: the vault's key material, written once by
  `rowel admin key-file <path>` from the master password (`0600`, raw bytes).
  Read again at every open, so an idle lock in this mode is lifted by the
  next request — the broker runs unattended. This is the mode for a systemd
  credential, which is a file under `$CREDENTIALS_DIRECTORY`.
- **`--key-fd <n>`**: the same material on an open file descriptor, read
  once at start — for a wrapper that redirects the file onto one
  (`rowel serve --key-fd 3 3< /run/secrets/vault.key`) or a supervisor that
  passes secrets as descriptors. The material is gone after the first seal,
  so an idle lock is refused in this mode.

A lockbox whose password change was interrupted (its `vault.db.rekey-backup`
is still beside the database) is refused: the app's next unlock rolls it
back; copy the directory again after.

## The config file

`<data dir>/serve.json`, or `--config <path>`. Missing means the defaults;
an unknown key is an error. Re-read when the file's mtime changes (checked
at most once a second), so `rowel admin rules add` takes effect on a running
broker.

```json
{
  "rules": [
    {
      "id": "3f9c…",
      "agents": { "ids": ["<client key>"] },
      "items": { "ids": ["<item id>"] },
      "targets": [{ "http": { "host": "api.stripe.com", "path": "/v1/*", "methods": ["GET"] } }],
      "modes": ["header"],
      "approval": "auto",
      "ttlMs": 900000,
      "maxUses": null,
      "reasonRequired": false,
      "approvers": null,
      "sidecarFor": null,
      "createdAtMs": 1761400000000,
      "createdBy": { "admin": "config" },
      "expiresAtMs": null
    }
  ],
  "askTimeoutMs": 120000,
  "idleLockMs": null,
  "listen": {
    "socket": true,
    "ws": "127.0.0.1:7401",
    "tls": { "cert": "/etc/rowel/cert.pem", "key": "/etc/rowel/key.pem" }
  }
}
```

- `rules`: `rowel_broker::policy::Rule` as serde writes it. `agents` is
  `"any"`, `{ "ids": [client keys] }`; `items` is `"lockbox"`, `{ "ids":
  [...] }`, `{ "tag": "…" }` or `{ "kind": "…" }`; `targets` are `{ "origin":
  … }` for `fill`/`passkey`, `{ "http": { host, path, methods } }` for
  `header`, `{ "command": "<full line>" }` for `env`; `approval` is `auto`,
  `ask` or `step_up`. The floors of spec `03` hold whatever the rule says:
  cards and identities are stepped up, passkeys always ask, and `env` asks
  unless the rule names the full command line. Every rule here is the
  operator's: `createdBy` reads as `{ "admin": "config" }` whatever the file
  says. The engine evaluates the baseline (any agent, any item, any target,
  `ask`, 15 minutes), then these, then the lockbox's own rules — the ones
  written by answering "for N days".
- `askTimeoutMs`: how long an ask waits for `rowel approve` before it is
  refused as `timeout` (default 120 s, spec `03 §4`). `--ask-timeout
  <seconds>` overrides.
- `idleLockMs`: seal after this long without a request; `null` never.
  `--idle-lock <minutes>` overrides.
- `listen.socket`: the data directory's socket, where the extension and the
  CLI on this box look for the app (`--no-socket` turns it off). The app
  cannot be serving it: the broker lock above refused one of the two first.
- `listen.ws`: the WebSocket address, or `null`; `--ws <addr>` overrides.
  `listen.tls`: PEM files for the certificate chain and the private key;
  `null` serves `ws://` in the clear, for a reverse proxy that terminates
  TLS.

The data directory also gets `broker.lock` (the one-broker lock above) and
`serve.sock` (the control socket below) while the broker runs.

## Approvals: `rowel approve`

An ask the rules do not settle is queued — approvers see one at a time, the
rest wait — and announced once on the broker's stderr with the command that
answers it:

```
rowel serve: [a1b2c3d4…] Claude Code (agent) asks to use "Stripe (live)" in header mode at GET api.stripe.com/v1/* — reason: deploy
             answer with: rowel approve a1b2c3d4… [--item <id>] [--days N | --run] | rowel approve a1b2c3d4… --deny
```

```
rowel approve [--data-dir <path>]                              # list what is waiting, one line per ask
rowel approve <id> [--item <id>] [--days N | --run]            # allow a grant: the item (needed when several match), for N days or this run
rowel approve <id> [--name <name>]                             # let a client in, under a name (default: the platform's label)
rowel approve <id> [--account <n>]                             # a passkey ceremony, as account n (default 0)
rowel approve <id> --deny
```

A grant ask marked stepped up prompts for the master password. `--days N`
writes a standing rule into the lockbox, as the app's "for N days" does.
`--run` is the app's "for this run", offered (`[--days N | --run]`) only when
the request named a run: further requests the agent marks as the same run are
served under the grant for at most 8 hours.
Every decision is an audit row in the lockbox: `requested`, `granted`,
`denied` with `user`, `timeout`, `lockbox` or `no_rule`, `used` for every
value a sidecar is served.

## `rowel admin`

```
rowel admin rules list [--config <path>]
rowel admin rules add --agent <client key|any> --items <lockbox|id,id|tag:<t>|kind:<k>>
                      --mode <fill|passkey|header|env> --target <origin | "[METHODS ]host/path" | command line>
                      [--approval auto|ask|step_up] [--ttl <minutes>] [--max-uses N] [--days N]
rowel admin rules remove <id>
rowel admin agents on|off [--lockbox <id>]      # the registry's flag; a running broker revokes on off
rowel admin key-file <path>                      # derive the key file from the master password
rowel admin lock                                 # seal a running broker
```

All take `--data-dir <path>`. `rules add` prints the new rule's id; `rules
list` prints each rule in words. The client key an `--agent` names is the
one `rowel status` reports for the agent or the sidecar on the box that will
connect, or the key an association ask announced.

## The WebSocket and TLS

Over the WebSocket a binary message carries exactly one frame's JSON body,
the sealed channel inside it unchanged; text messages are ignored. Plain
`ws://` is for the loopback address or a reverse proxy; `wss://` with
`listen.tls` serves TLS from the PEM files directly. On the agent's side set
`ROWEL_BROKER=ws://host:port` or `wss://…` (and `ROWEL_BROKER_CA` for a CA
of your own) before `rowel mcp`, `rowel run`, `rowel proxy` — see
[`cli.md`](cli.md#reaching-a-broker). The two keys are let in the first time
each is seen, through an association ask answered with `rowel approve`.

Anyone who can reach the port can open a connection and hold an association
ask for its timeout, so what a stranger can cost the broker is bounded:
at most 256 connections are open at once (one past that is closed before
the handshake, and the agent retries), a handshake has ten seconds to
finish, a message may carry no more than the broker's 1 MiB frame, and each
connection has a queue of four requests after which the broker stops
reading from it until its thread catches up. A connection's place under the
ceiling belongs to the thread serving it, not the socket: hanging up with
an ask in flight keeps the place taken until the ask is answered or times
out, so hanging up and reconnecting is not a way past the ceiling. Put the listener on the
loopback address or behind a reverse proxy that authenticates, as with any
service; the ceiling keeps a flood from taking the process down, it does
not keep a flood from occupying the connections an agent would use.

## The browser extension on the box

The broker serves the same socket the app does, so the Rowel extension in a
browser on the box can fill from it — once the browser can find it. A
browser finds a native messaging host by a manifest in a directory of its
own, which the app writes for itself; on a box without the app, the broker
writes it:

```bash
rowel serve --install-host [--data-dir /srv/rowel]     # register, then exit
rowel serve --uninstall-host [--data-dir /srv/rowel]   # take it back
```

`--install-host` writes `native-host` into the data directory — a launcher,
executable by you alone, that runs `rowel native-host --data-dir <dir>`, the
relay between the browser's stdio and the broker's socket — and a manifest
naming it, `app.rowel.browser.json`, for every browser that has run here:
Chrome, Chromium, Edge, Brave and Vivaldi under their own directories
(`~/.config/<browser>/NativeMessagingHosts/` on Linux, `~/Library/Application
Support/<browser>/NativeMessagingHosts/` on macOS) and Firefox under
Mozilla's (`~/.mozilla/native-messaging-hosts/`, `~/Library/Application
Support/Mozilla/NativeMessagingHosts/`). The manifest lets in the Rowel
extension's ids and nothing else. Start the browser and install the
extension first, so its directory exists; then run `--install-host`, and
again after moving the binary or the data directory. It serves nothing and
takes only `--data-dir`; `rowel serve` itself is started as usual, and the
extension connects while it runs.

A manifest the app wrote (or another program) under the same name is left
alone and reported, and the app leaves the broker's alone in turn: on a
machine with both, whichever registered first is the one the browser
launches. Both relays reach the socket in the app's data directory, which is
`rowel serve`'s default, so either works there; a broker on another
`--data-dir` needs its own. Windows is not supported: browsers find a host
there through the registry, and the app registers itself. A browser
installed as a Snap or a Flatpak may not be allowed to launch a host outside
its sandbox.

## A systemd unit

```ini
[Unit]
Description=Rowel headless broker
After=network.target

[Service]
User=rowel
ExecStart=/usr/local/bin/rowel-cli serve --data-dir /var/lib/rowel --key-file ${CREDENTIALS_DIRECTORY}/vault.key --ws 127.0.0.1:7401
LoadCredential=vault.key:/etc/rowel/vault.key
StandardInput=null
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

`LoadCredential` copies the key file into a private directory the service
alone can read, named by `$CREDENTIALS_DIRECTORY` in its environment, which
systemd expands in `ExecStart`; the broker reads it there at every open, so
the idle lock works, and the directory goes when the service stops.
`SIGTERM` from `systemctl stop` seals the lockbox and exits 0, whatever the
broker is doing at the time, including waiting at a password prompt.

## What is not there

- No GUI, and no biometrics: step-up is the master password at `rowel
  approve`.
- No sync: the lockbox is served as copied. Items added on the device
  reach the box when the directory is copied again.
- No `generate-password`: the generator's settings are the app's; the
  extension handles the refusal.
- No tenants, members or workload identity: that is the Rowel Server, a
  separate product this repository holds only the clients of.

What a Rowel Server speaks, as the app and the CLI already speak it, is [`docs/server-api.md`](./server-api.md).
