# Running a Rowel Server

A Rowel Server is the Rowel broker in server mode: one binary and one data
directory a company runs inside its own network. It holds **org lockboxes**,
lets the company's people hold them in their Rowel apps, and serves them to
the company's agents — a GitHub Actions job, a deploy bot — under policy,
routing what policy does not settle to a member's app for a tap. It is the
`rowel` command line's `serve` in server mode:

```bash
rowel serve --server --data-dir /var/lib/rowel
```

The same grant engine, sealed channel and audit chain as the app and the
headless broker ([`serve.md`](serve.md)) decide here; what is new is who is
asking. Members sign in through the company's identity provider and enroll
their devices; agents prove what they are with the token their platform
mints (GitHub Actions' OIDC token, a Kubernetes service account token), and a
trust policy says which agent that token is. What the server speaks, to the
app, the CLI and the extension, is [`server-api.md`](server-api.md).

## What it can read, and what it cannot

**It can open the org lockboxes.** Each one's key is a random master the
server minted when the lockbox was created, kept in its database wrapped
under the **root key**. That is what lets an agent be served at 3 a.m. with
nobody's password typed: the server unwraps the master, opens the lockbox and
answers under policy. Every member device that holds the lockbox gets the
same master sealed to that device's own enrollment key, and only after the
device was approved. The app says so when a member adds an org lockbox.

**It cannot open anyone's personal lockboxes.** A member's own lockboxes are
keyed from their master password and never leave their devices or their own
cloud account; the server never sees them, their keys, or the password. An
agent of the company's reaches only the lockbox its definition is bound to.

**No value goes to an agent.** As everywhere in Rowel, a client of kind
`agent` gets a grant id and an outcome; the value goes to a sidecar (`rowel
run`, `rowel proxy`) that puts it into one command or one request.

## Setting one up

Everything below is `rowel admin server …` on the data directory. It works
whether or not the server is running: the metadata is SQLite in WAL mode,
and the running server reads rules and trust policies at every request, so a
change is in force at the next one. Run it as the user the server runs as,
so every file in the data directory stays that user's. Every command takes
`--data-dir <path>` (there is no default: a server has no conventional
place) and `--config <path>` when `server.json` is not in the data directory.

1. **Initialize.** Writes `server.json`, mints the root key, creates the
   database and the tenant, and says what to do next:

   ```bash
   rowel admin server init --data-dir /var/lib/rowel --tenant "Acme" \
       --public-url https://rowel.acme.example --root-key /etc/rowel/root.key
   ```

   Without `--root-key` the key is minted as `root.key` *in* the data
   directory, which is fine for a try on a laptop and wrong for anything
   else: move it out (below) before the directory is ever backed up. A
   second `init` is refused: a root key is never overwritten, since every
   master in the database is wrapped under the one that is there. With a
   `server.json` already in place, `init` keeps it and mints the key it
   names.

2. **Add yourself as an admin.**

   ```bash
   rowel admin server members add you@acme.example --name "Your Name" --admin --data-dir /var/lib/rowel
   ```

3. **Point the identity provider** at the server and fill in
   `members.oidc` (below).

4. **Create a lockbox and give it holders.**

   ```bash
   rowel admin server lockboxes create Deploy --data-dir /var/lib/rowel      # prints its id
   rowel admin server lockboxes members add <lockbox id> you@acme.example --role admin --data-dir /var/lib/rowel
   ```

5. **Run it**: `rowel serve --server --data-dir /var/lib/rowel`, under
   systemd and behind a reverse proxy as below. It refuses a data directory
   `init` never made into a server, and a second process on the same one —
   another server, or `rowel serve` — exits 1 while the first runs. It
   says `listening on http(s)://<address>` on stderr, warns when it serves
   plain http to anything but this machine, and answers `GET /v1/health`
   with `{"ok":true}` and nothing else, without a sign-in, for a load
   balancer or a monitor. SIGTERM or Ctrl-C stops it: open connections get
   five seconds, every open lockbox is sealed and its master wiped, and it
   exits 0 after `sealed, exiting`.

6. **Put items in the lockbox** from the app: in Rowel, Settings ›
   Lockboxes › **Rowel Server**, connect to `https://rowel.acme.example`,
   sign in, approve the device, **Add** the lockbox and fill it as any other.
   The app syncs it through the server.

7. **Define an agent** and its trust policy, turn the lockbox's agent access
   on, and write the rules it runs under (below).

## `server.json`

Read once at start (restart the server to change it) and read strictly: a
key the server does not know is an error naming the file, and a field that
does not check out is an error naming the field. Rules are not here; they
are in the database, where `rowel admin server rules` puts them.

```json
{
  "tenant": { "name": "Acme" },
  "publicUrl": "https://rowel.acme.example",
  "listen": {
    "http": "127.0.0.1:7402",
    "tls": null
  },
  "rootKeyFile": "/run/credentials/rowel-server.service/root.key",
  "members": {
    "oidc": {
      "issuer": "https://login.acme.example",
      "clientId": "rowel",
      "clientSecret": "…",
      "allowedDomains": ["acme.example"]
    }
  },
  "askTimeoutMs": 120000,
  "keyIdleMs": 600000,
  "sessionTtlMs": 2592000000
}
```

- `tenant.name`: the company's name, as the app shows it beside an org
  lockbox and on an approval routed from here. Default `Rowel`. The tenant's
  id is minted once into the database and never changes; the name follows
  this file.
- `publicUrl`: what the app and the agents are given to reach the server,
  and what redirects are built from. `https`, or `http` to a loopback host
  only (`localhost`, `127.0.0.0/8`, `[::1]`): the app refuses anything else
  before it sends a request, so a server offering it could not be reached.
  Normalized on load (lower case, no default port, no trailing slash).
  Default `http://127.0.0.1:7402`. It must be exactly the address members
  connect the app to, because enrollment proves it: every enrollment
  challenge names this URL, and the app answers only one naming the server
  it signed in to (`docs/server-api.md`, "Member API"). A server members
  reach under another name, or behind a proxy whose address is not this one,
  enrolls no device.
- `listen.http`: the one listener, `host:port`, for every API and the agent
  WebSocket together. Default `127.0.0.1:7402`.
- `listen.tls`: `{ "cert": "<PEM chain>", "key": "<PEM key>" }` to serve TLS
  directly; `null` serves in the clear, for a reverse proxy that terminates
  TLS in front — `publicUrl` then names the proxy's `https` address.
- `rootKeyFile`: the 32-byte root key file. The one field with no default.
- `members.oidc`: the company's identity provider. Absent, no member can
  sign in; `rowel admin server` still works, which is how the first admin is
  made. `issuer` is the provider's https issuer URL (its discovery document
  is read from it); `clientId` and `clientSecret` are the server's client
  there; `allowedDomains` are the email domains a person signing in may
  belong to (below). The file holds the client secret, so it is written
  `0600` and should stay so.
- `askTimeoutMs`: how long an ask routed to members waits for an answer
  before it is a no (default 120 s).
- `keyIdleMs`: how long an unwrapped lockbox master stays in memory unused
  before it is dropped (default 10 minutes); a quiet server holds no
  plaintext keys.
- `sessionTtlMs`: how long a member's session lives before they sign in
  again (default 30 days).

## The root key

Thirty-two random bytes in a file, minted by `init`, readable by its owner
alone (`0600`; a key file anyone else can read is refused at start). The
tenant key every lockbox master is wrapped under is derived from it, so:

- **Hold it apart from the data directory.** A copy of the data directory
  without the key opens nothing; with the key beside it, it opens every org
  lockbox. Keep the key outside the directory and outside the directory's
  backups.
- **Back it up separately**, somewhere the backups of the data directory are
  not. Lose the key and every org lockbox on the server is gone: the
  members' devices keep their copies, but nothing can wrap a master for a new
  device or serve an agent again.
- **Never mint over it.** `init` refuses to; there is no command that
  replaces it in this release.

`lockboxes create` is the one admin command that needs the key, to wrap the
new master: it reads `rootKeyFile`, or `--root-key <path>` when the config
names a path only the running service can read (as under systemd, below).

## Members and the identity provider

The server is two things to sign-in: the **OAuth2 authorization server** of
the Rowel app (the app signs in to the server with PKCE and holds the
server's tokens, never the provider's), and an **OpenID Connect relying
party** of the company's identity provider (the server sends the browser
there and takes the answer). At the provider, register the server as a web
application client using the authorization code flow, with scopes `openid
email profile`, and this redirect URI:

```
https://rowel.acme.example/v1/members/auth/callback
```

— `<publicUrl>/v1/members/auth/callback`, which `init` prints. Put the
issuer, the client id and the client secret in `members.oidc` and restart.

Who may become a member: with `allowedDomains` set, anyone the provider
vouches for with an email in one of those domains becomes a member at their
first sign-in. With it empty, only the people an admin added can sign in —
the right setting for a provider that hands out accounts to the public.

```
rowel admin server members add <email> [--name <name>] [--admin]
rowel admin server members list
rowel admin server members disable <email|id>        # and ends their sessions
```

Members are named by email or id wherever one is taken.

## Devices and approval

When a member signs in from the app, the app enrolls the computer as one of
their devices under its host name, with an X25519 enrollment key it made.
The device is **pending** until an approved device of the same member, or an
admin, approves it; until then the server seals no master to it, so a stolen
sign-in alone opens nothing.

```
rowel admin server devices list --member <email|id>   # id, name, state, fingerprint, enrolled
rowel admin server devices approve <id>
rowel admin server devices revoke <id>
```

The fingerprint is the one the app shows on the new device's screen (four
groups of four hex digits): read it to the member, or have them read it to
you, before you approve. A revoked device loses every master sealed to it at
once and is never approved again; the member enrolls it afresh.

## Lockboxes

```
rowel admin server lockboxes create <name> [--root-key <path>]
rowel admin server lockboxes list                     # id, name, gen, frozen, agents, revision
rowel admin server lockboxes freeze|unfreeze <id>
rowel admin server lockboxes agents on|off <id>
rowel admin server lockboxes members add <lockbox> <email|id> --role reader|approver|admin
rowel admin server lockboxes members remove <lockbox> <email|id>
rowel admin server lockboxes members list <lockbox>
```

A lockbox is a SQLCipher file under `lockboxes/<id>/` in the data directory,
the same file the app writes, keyed by its master. Its id is its vault id on
every device that holds it. `gen` is its key generation, `revision` moves
with every write of the file (it is the pack's ETag). **Frozen**, nothing is
served from it whatever policy says. **Agents** is its agent-access switch,
off at creation, as in the app.

Roles: a **reader** holds the lockbox on their devices; an **approver** also
answers agents' asks for it; an **admin** of the lockbox also administers it.
Asks are routed to its approvers and admins.

## Agents and trust policies

An **agent** is a definition: a name and the one lockbox it is bound to.
A **trust policy** says which workload tokens become that agent: the issuer
whose tokens count, and the claims every one of them must carry.

```
rowel admin server agents add <name> --lockbox <id>
rowel admin server agents list                        # each with its trust policies and the kind each admits
rowel admin server agents enable|disable|remove <id>
rowel admin server trust add --agent <id> --issuer <url> --claim <name>=<pattern> [--claim …] [--fixed-audience] [--sidecar]
rowel admin server trust remove <id>
```

A trust policy admits one kind of client. Without `--sidecar` a matching
token becomes the **agent**, which asks for grants and never gets a value;
with it, the agent's **sidecar** (`rowel proxy`, `rowel run`), the one kind
that resolves a placeholder into the value. A token is checked only against
the policies of the kind its client says it is, so a token that matches the
agent's policy alone cannot associate as the sidecar. Give each the claims
of its own workload; see [A sidecar with its own identity](#a-sidecar-with-its-own-identity).

A claim's pattern is exact unless it begins with `glob:`, where `*` matches
any run of characters (`ref=glob:refs/heads/release/*`). A claim that is not
a string in the token is compared as its JSON text, so `ref_protected=true`
matches a boolean. At least one claim is required: an issuer like GitHub's
mints tokens for every repository in the world, and a policy with no claims
would make all of them this agent.

`--fixed-audience` is for a platform that cannot put the session's key in
the token's audience (a Kubernetes projected token): the token names the
server's bare audience, `rowel:<tenant id>` (`GET
/v1/agent/challenge` answers it), lives at most ten minutes, carries a `jti`
the server remembers, and the challenge nonce echoed inside the sealed
channel binds it to the connection. GitHub Actions can choose the audience,
so it does not need the flag: the CLI asks for a token whose audience ends in
the digest of this connection's session key, and a token lifted from one
connection is refused on every other.

Disabling an agent refuses its clients at their next request; removing it
removes its trust policies and the clients it became.

### A GitHub Actions job

```bash
rowel admin server agents add "api deploy" --lockbox <lockbox id> --data-dir /var/lib/rowel   # prints the agent id
rowel admin server trust add --agent <agent id> \
    --issuer https://token.actions.githubusercontent.com \
    --claim repository=acme/api --claim ref=refs/heads/main --data-dir /var/lib/rowel
# The proxy runs in the same job, so it carries the same token: see below.
rowel admin server trust add --agent <agent id> --sidecar \
    --issuer https://token.actions.githubusercontent.com \
    --claim repository=acme/api --claim ref=refs/heads/main --data-dir /var/lib/rowel
rowel admin server lockboxes agents on <lockbox id> --data-dir /var/lib/rowel
rowel admin server rules add --agent <agent id> --items lockbox --mode header \
    --target "GET api.stripe.com/v1/*" --lockbox <lockbox id> --data-dir /var/lib/rowel
```

```yaml
jobs:
  deploy:
    runs-on: ubuntu-latest
    permissions:
      id-token: write        # the runner may mint the OIDC token the server checks
      contents: read
    env:
      ROWEL_BROKER: wss://rowel.acme.example/v1/agent
    steps:
      - uses: actions/checkout@v4
      # rowel-cli from the release, on PATH as rowel.
      - name: Rowel proxy
        run: rowel proxy &   # the sidecar: puts the key into requests at 127.0.0.1:7400
      - name: Deploy
        run: ./deploy.sh     # an agent asks with rowel_request_secret, calls the API through the proxy
```

The first request under each of the job's two keys carries the job's token
as proof; the server verifies it against GitHub's keys, matches the policy,
and the job is the agent `api deploy`. Under the rule above, a header grant
for `GET api.stripe.com/v1/*` is `auto`; anything the rules leave to the
baseline is asked of the lockbox's approvers, in their Rowel apps, and the
job waits for the tap or the timeout. How the CLI gets and binds the token
is in [`cli.md`](cli.md#proving-identity-to-a-rowel-server); the proxy and
`rowel run` in [`cli.md`](cli.md#rowel-proxy).

### A sidecar with its own identity

The agent is what a prompt injection reaches, and the sidecar is what holds
values, so the split between them is worth as much as the platform's ability
to tell them apart. The server can only go by the token: whatever runs under
the sidecar's platform identity can associate as the sidecar.

- **Kubernetes.** Every container in a pod shares the pod's service account
  and gets the same projected token, so a sidecar container beside the
  agent cannot have an identity of its own. Run the proxy as its own
  Deployment under its own service account, reached by the agent over the
  pod network (a Service), and write the sidecar's policy on that account
  (`--claim sub=system:serviceaccount:deploy:rowel-proxy --sidecar
  --fixed-audience`) and the agent's on the agent's.
- **GitHub Actions.** Every step of a job gets the same token. A proxy in a
  job of its own — a reusable workflow, whose token's `job_workflow_ref`
  names it (`--claim job_workflow_ref=acme/ci/.github/workflows/rowel-proxy.yml@refs/heads/main
  --sidecar`) — has an identity of its own, but it must run where the
  agent's job can reach it over the network, which on GitHub's hosted
  runners it cannot; it takes self-hosted runners on one network.

**What is left when they share an identity.** When the agent and the sidecar
run in the same job or the same pod under the same platform identity, as in
the job above, the platform cannot tell them apart, and neither can the
server: the agent could associate as the sidecar and resolve values itself.
The two policies then admit the same tokens, and the separation is only as
strong as the platform's identities. What still holds is everything after
association: a value is released only under a grant the rules or a member
allowed, for the item, place and mode it names, and every release is in the
audit log.

## Rules

```
rowel admin server rules add --agent <agent id|any> --items <lockbox|id,id|tag:<t>|kind:<k>>
                             --mode <fill|passkey|header|env> --target <origin | "[METHODS ]host/path" | command line>
                             [--approval auto|ask|step_up] [--ttl <minutes>] [--max-uses N] [--days N]
                             [--lockbox <id>]
rowel admin server rules list
rowel admin server rules remove <id>
```

The flags are `rowel admin rules add`'s ([`serve.md`](serve.md#the-config-file)
has the shapes), with two differences: `--agent` names an agent definition's
id, which covers every run of that agent, and `--lockbox` makes the rule
that lockbox's; without it the rule holds for every lockbox of the tenant.
`rules list` prints each rule in words with the lockbox it is for. The floors
of spec `03` hold whatever a rule says: cards and identities are stepped up,
passkeys always ask, and `env` asks unless the rule names the full command
line.

## Audit

Every decision is a row in the lockbox's own audit chain, inside the file,
so a lockbox restored elsewhere carries its history; the server mirrors the
rows into its database as they are written, so an export covers the tenant
without opening each lockbox.

```
rowel admin server audit export [--since <time>] [--until <time>] [--lockbox <id>] [--actor <id>]
rowel admin server audit verify
```

`export` writes JSON Lines on stdout, one row per line, in chain order
(lockbox, writer, sequence): `lockboxId`, `writerId`, `seq`, `id`, `tsMs`,
`prevHash` and `hash` in hex, `actorKind`, `actorId`, `actorName`, `runId`,
`actingFor`, `grantId`, `itemId`, `target`, `mode`, `approval`, `approver`,
`event`, and `detail` as the text the row was hashed over. A time is RFC 3339
(`2026-10-01T00:00:00Z`) or milliseconds since the epoch; `--since` is
inclusive and `--until` exclusive. `--actor` is an agent's or a member's id.
An empty mirror prints nothing.

`verify` walks every writer's chain in every lockbox and prints `ok` with
the count, or the first break in words — a missing row, a row that does not
follow the one before it, a row that was altered, two rows with the same
number — naming the lockbox and the writer, and exits 1.

## A systemd unit

A dedicated user rather than `DynamicUser`, because `rowel admin server` runs
as the same user against the same files:

```bash
useradd --system --home-dir /var/lib/rowel --shell /usr/sbin/nologin rowel
install -d -o rowel -g rowel -m 0700 /var/lib/rowel /etc/rowel
sudo -u rowel rowel admin server init --data-dir /var/lib/rowel --tenant Acme \
    --public-url https://rowel.acme.example --root-key /etc/rowel/root.key
```

Then set `rootKeyFile` in `/var/lib/rowel/server.json` to the credential
path below, and pass `--root-key /etc/rowel/root.key` to `lockboxes create`.

```ini
[Unit]
Description=Rowel Server
After=network-online.target
Wants=network-online.target

[Service]
User=rowel
Group=rowel
ExecStart=/usr/local/bin/rowel-cli serve --server --data-dir /var/lib/rowel
LoadCredential=root.key:/etc/rowel/root.key
StateDirectory=rowel
StateDirectoryMode=0700
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
NoNewPrivileges=yes
StandardInput=null
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

`LoadCredential` copies the key into a private directory only the service
can read, `$CREDENTIALS_DIRECTORY` in its environment — for a unit named
`rowel-server.service`, `/run/credentials/rowel-server.service/root.key`,
which is what `rootKeyFile` names (the config is not expanded, so write the
path out). The copy goes when the service stops. `ProtectSystem=strict`
leaves the service nothing writable but the `StateDirectory`, which is the
data directory.

## Behind a reverse proxy

Run the server on the loopback address (`listen.http` `127.0.0.1:7402`,
`listen.tls` `null`) and let the proxy hold the certificate. It must pass the
WebSocket upgrade on `/v1/agent`, never buffer the approvals stream (SSE),
and take a lockbox pack as a request body, up to 256 MiB.

Caddy (certificates, WebSockets and streaming are handled for you):

```
rowel.acme.example {
    request_body {
        max_size 256MB
    }
    reverse_proxy 127.0.0.1:7402 {
        flush_interval -1
    }
}
```

nginx:

```nginx
server {
    listen 443 ssl;
    server_name rowel.acme.example;
    ssl_certificate     /etc/ssl/rowel/fullchain.pem;
    ssl_certificate_key /etc/ssl/rowel/privkey.pem;

    client_max_body_size 256m;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-Proto https;

    location / {
        proxy_pass http://127.0.0.1:7402;
    }

    location = /v1/agent {
        proxy_pass http://127.0.0.1:7402;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_read_timeout 1h;
    }

    location = /v1/members/me/approvals/stream {
        proxy_pass http://127.0.0.1:7402;
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_buffering off;
        proxy_read_timeout 1h;
    }
}
```

## Backup and restore

A backup is three things, the third kept apart from the other two:

1. **The metadata database**, `server.db`. Copy it with SQLite's own backup
   rather than the file, since a running server may have pages in the WAL:
   `sqlite3 /var/lib/rowel/server.db ".backup '/backup/server.db'"`.
2. **The lockbox files**, `lockboxes/` in the data directory, and
   `server.json`.
3. **The root key**, backed up once, separately, never beside 1 and 2.

To restore: stop the server; put the root key back where `rootKeyFile` says
(owner-only); put `server.json`, `server.db` and `lockboxes/` back in the
data directory, owned by the service user; start the server and run `rowel
admin server audit verify`. Members' devices push what they wrote while the
server was away at their next sync, and the server merges it.

## What is not in this release

- **Key rotation and offboarding.** A lockbox stays at its first key
  generation, and disabling a member ends their sessions but does not
  rotate the lockboxes they could read.
- **Administration in the app.** Admin-role members manage the server from
  the command line here; the screens in Settings come later.
- **Postgres.** The metadata is SQLite on one node.
- **Approvals pushed to a phone.** An ask reaches members' Rowel apps on
  their desktops while those are unlocked.
