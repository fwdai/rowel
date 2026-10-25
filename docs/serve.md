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
directory, readable by the service user alone, and prefer `--key-fd` under a
service manager that hands credentials as descriptors. Nothing is served
from a sealed lockbox; `SIGTERM`, `rowel admin lock` and the idle lock seal
it, and every grant goes with the seal.

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
   `workspaces/<id>` in it beside `workspaces.json`. The install id file
   travels with it, so the audit chain the device started continues on the
   box; the vault is served as it was copied — the broker does not sync.
3. Install the CLI binary (`rowel-cli`, from the release, or
   `cargo build -p rowel-cli`), reached as `rowel` below.
4. Unlock and serve:

   ```bash
   rowel serve --data-dir /srv/rowel          # asks for the master password
   ```

   The lines on stderr say which lockbox is open and where it listens.

## Unlock modes

- **Password** (default): asked for on the terminal without echo, or read
  as one line from stdin when that is not a terminal. After an idle lock the
  terminal is asked again.
- **`--key-file <path>`**: the vault's key material, written once by
  `rowel admin key-file <path>` from the master password (`0600`, raw bytes).
  Read again at every open, so an idle lock in this mode is lifted by the
  next request — the broker runs unattended.
- **`--key-fd <n>`**: the same material on a file descriptor, read once at
  start — what `systemd`'s `LoadCredential=` hands a service. The material is
  gone after the first seal, so an idle lock is refused in this mode.

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
  CLI on this box look for the app (`--no-socket` turns it off). Before
  binding, the broker probes the socket: if the app is serving this data
  directory, it refuses to start rather than take the socket over.
- `listen.ws`: the WebSocket address, or `null`; `--ws <addr>` overrides.
  `listen.tls`: PEM files for the certificate chain and the private key;
  `null` serves `ws://` in the clear, for a reverse proxy that terminates
  TLS.

## Approvals: `rowel approve`

An ask the rules do not settle is queued — approvers see one at a time, the
rest wait — and announced once on the broker's stderr with the command that
answers it:

```
rowel serve: [a1b2c3d4…] Claude Code (agent) asks to use "Stripe (live)" in header mode at GET api.stripe.com/v1/* — reason: deploy
             answer with: rowel approve a1b2c3d4… [--item <id>] [--days N] | rowel approve a1b2c3d4… --deny
```

```
rowel approve [--data-dir <path>]                              # list what is waiting, one line per ask
rowel approve <id> [--item <id>] [--days N]                    # allow a grant: the item (needed when several match), for N days
rowel approve <id> [--name <name>]                             # let a client in, under a name (default: the platform's label)
rowel approve <id> [--account <n>]                             # a passkey ceremony, as account n (default 0)
rowel approve <id> --deny
```

A grant ask marked stepped up prompts for the master password. `--days N`
writes a standing rule into the lockbox, as the app's "for N days" does.
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

## A systemd unit

```ini
[Unit]
Description=Rowel headless broker
After=network.target

[Service]
User=rowel
ExecStart=/usr/local/bin/rowel-cli serve --data-dir /var/lib/rowel --key-fd 3 --ws 127.0.0.1:7401
LoadCredential=vault.key:/etc/rowel/vault.key
StandardInput=null
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

`LoadCredential` opens the key file as a descriptor the service may read
once; `--key-fd 3` is that descriptor when the credential is the first and
stdio is not redirected elsewhere — check `CREDENTIALS_DIRECTORY` in your
unit's environment if the number differs. `SIGTERM` from `systemctl stop`
seals the lockbox and exits 0.

## What is not there

- No GUI, and no biometrics: step-up is the master password at `rowel
  approve`.
- No sync: the lockbox is served as copied. Items added on the device
  reach the box when the directory is copied again.
- No `generate-password`: the generator's settings are the app's; the
  extension handles the refusal.
- No server mode yet (`--server`, tenants, workload identity): that is the
  Rowel Server of spec `07`.
