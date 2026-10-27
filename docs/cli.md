# The command line: `rowel-cli`

Rowel ships a small command line beside the app, `rowel-cli` (the crate is
`src-tauri/crates/cli`). It talks to the running app over the same socket and
sealed channel the browser extension uses, as two clients: an *agent* (`rowel
mcp`), which can ask for grants and nothing else, and a *sidecar* (`rowel run`,
`rowel redact`, `rowel hook post`), the one client that can resolve a secret
under a grant — into one command's environment, or out of a stream, never to
the agent. Settings › General › Command line tool puts it on your `PATH` as
`rowel` and shows its full path ([Installing](#installing)). Below, `rowel`
stands for whichever name you have.

## Installing

The app bundles the tool; **Install** in Settings › General › Command line
tool makes it `rowel`:

- **macOS, and Linux from a `.deb` or `.rpm`**: a symlink named `rowel` to
  the bundled `rowel-cli`, in `/usr/local/bin` when you can write there, else
  in `~/.local/bin` (made if missing). The link follows the app through an
  update. A `rowel` there that is not a link to a `rowel-cli` is left alone
  and the install refused. When the folder is not on the `PATH` a new
  terminal gets (on macOS `/etc/paths`, which lists `/usr/local/bin` but not
  `~/.local/bin`), the row says to add it.
- **Linux from an AppImage**: the bundled binary is inside the image's mount,
  which is gone when the app quits, so at every start the app copies it to
  `~/.local/share/app.rowel.desktop/bin/rowel-cli` (only when it changed),
  and the link points at that copy.
- **Windows**: the folder that holds the bundled `rowel-cli.exe` also holds
  the app's own `rowel.exe`, so it cannot go on the `PATH`. Rowel copies the
  tool to `%LOCALAPPDATA%\app.rowel.desktop\bin\rowel.exe` and adds that
  folder to your user `PATH` (`HKEY_CURRENT_USER\Environment`), then tells
  Windows the environment changed: terminals opened after that find
  `rowel`, ones already open do not. At every start the app refreshes the
  copy when an update brought a new one, moving aside a copy an agent is
  still running.

**Remove** takes the link, or the `PATH` entry and the copy, away again.
Uninstalling the app on Windows does not edit your `PATH`: remove the tool
first, or delete the entry afterwards.

The row shows the full path of the tool, with a copy button: the `rowel` on
your `PATH` once it is installed, the bundled binary before. An MCP config
should name the tool by that path. An agent the Dock, the Start menu or a
desktop launcher started does not read your shell's `PATH` (a macOS app gets
`/usr/bin:/bin:/usr/sbin:/sbin`), so a bare `rowel` is found only by an agent
started from a terminal.

Without the app — a CI runner, a server — download the standalone archive
from the release instead ([`serve.md`](serve.md#installing-the-command-line-tool)).

## Placeholders

An agent names a secret, it never holds one:

```
rowel://<lockbox>/<item>/<field>
```

- `lockbox` — the open lockbox's id, or its name as a slug (`Work (2026)` is
  `work-2026`: lower case, every run of characters that is not a letter or
  digit becomes one `-`).
- `item` — the item's id, as `rowel_list_visible` returns it, or its title as
  a slug when that is unique in the lockbox.
- `field` — `password` or `otp` of a login, `apiKey` of an API key,
  `privateKey` of an SSH key, `body.<KEY>` for one variable of an env item,
  `number` or `cvc` of a card.

A placeholder is not a capability: knowing the name of a secret buys nothing
without a grant that covers it. What makes a grant is the user's answer to
Rowel's dialog (or a standing rule they wrote by answering "for N days").

## The two keys

The first tool call of `rowel mcp` creates this machine's **agent key**; the
first request of `rowel run`, `rowel redact`, `rowel hook post` or `rowel
proxy` creates the **sidecar key**. Each
record is the public key and the name the app gave the client, and each asks
the app to let it in once — Rowel shows the dialog and you give the client a
name. Two keys because the host refuses `resolve` to any client that is not a
sidecar: the model-facing surface (`rowel mcp`) holds a key that cannot
resolve, whatever it is asked to do.

Where they live:

- **macOS**: two generic passwords in the login keychain, service
  `app.rowel.cli`, accounts `agent` and `sidecar`.
- **Windows**: two Credential Manager entries under the same names.
- **Linux and everything else**: files readable by you alone,
  `~/.config/rowel/agent.json` and `sidecar.json` (`$XDG_CONFIG_HOME` when
  set).

The key is only a public one, but it is the credential: whoever presents it
is this client. The secure store is not gated by Touch ID or Hello — the CLI
runs unattended — and what it adds over a file differs by platform:

- **macOS**: the keychain item's access list names the `rowel-cli` that made
  it, so another program reading it makes macOS prompt you first.
- **Windows**: Credential Manager has no per-program access list: any
  program running as you reads the entry without a prompt. What it adds is
  encryption at rest under your Windows account, which a file under your
  profile lacks.
- **Linux**: a file readable by you alone, as above.

So a same-user process can lift the key on every platform, without being
asked on Windows and Linux (`spec/10` Q9); grants, floors,
full-command-line targets and the rate limits bound what a lifted key buys.

A key written as a file by an earlier release is moved into the store the
first time it is read, and the file is overwritten with a one-line note,
`rowel-cli: this key is kept in <store>`, so that a later run whose store
cannot be read reports an error instead of minting a second key over the
first. A store that refuses (a locked or damaged keychain; anything but "no
such item") is stood in for by the file for that run, with one line on stderr
saying so, so a broken keychain never strands an association; the move is
tried again next run.

## Reaching a broker

By default every command talks to the app on this machine, over the socket in
its data directory. Set `ROWEL_BROKER` to reach a broker elsewhere instead —
a `rowel serve` ([`serve.md`](serve.md)) on a CI runner or a dev VM:

- unset or empty: the app's socket, as today;
- `wss://host:port[/path]`: a WebSocket to that address over TLS. The certificate is verified
  against the platform's roots; a broker signed by a CA of your own needs
  `ROWEL_BROKER_CA=<path to a PEM>`, which adds that CA beside them;
- `ws://localhost:port[/path]` (or `127.0.0.1`, `[::1]`): the same in the
  clear, to a broker on this machine only. The channel's key exchange is not
  authenticated, so in the clear across a network whoever sits in the middle
  could swap the keys and read every value resolved through it; `ws://` to
  any other host is refused. A broker behind a reverse proxy is reached over
  the proxy's `wss://`, and the proxy speaks `ws://` to it over loopback.

Any other value is refused by every command with a message naming the
variable and the forms it takes. The channel is the same sealed one whichever
way it goes, and so are the two keys: the broker asks to let each one in the
first time it sees it, as the app does. `rowel status` says which it tried
(`app:` or `broker: … at ws://…`).

## Proving identity to a Rowel Server

A job in CI has no one at the dialog. When `ROWEL_BROKER` names a Rowel
Server and the platform the CLI runs on can say what the job is, the first
`associate` under each key carries that statement as a *proof*, and the
server's trust policy decides instead of a person (`spec/02`, workload
identity). The CLI first asks the server for a challenge at
`https://<host>/v1/agent/challenge` (`http://` for a `ws://` broker), then
gets a token from one of two sources, tried in this order:

- **GitHub Actions.** With `ACTIONS_ID_TOKEN_REQUEST_URL` and
  `ACTIONS_ID_TOKEN_REQUEST_TOKEN` set, which the runner does for a job that
  may mint id tokens, the CLI asks the runner for a token whose audience is
  the server's audience followed by the SHA-256 of this connection's session
  key, so the token proves this channel and no other. The job needs:

  ```yaml
  permissions:
    id-token: write
  ```

- **A token file.** `ROWEL_WORKLOAD_TOKEN_FILE=<path>` sends the file's
  contents, trimmed, as the token, with the server's challenge nonce beside
  it: for a platform that mints tokens for a fixed audience, such as a
  Kubernetes projected service account token.

With neither set the association asks as before, answered with
`rowel approve`. A source that is set but cannot be read, or a server that
answers no challenge, is an error the command reports rather than a quiet
fall back to the ask: in CI nobody would answer it. The token is never
printed or logged, and is never sent to the app on this machine.

## `rowel serve`, `rowel approve`, `rowel admin`

The same binary is the broker on a box without the app:

```
rowel serve [--data-dir <path>] [--lockbox <id>] [--config <path>] [--key-file <path> | --key-fd <n>]
            [--ws <addr>] [--no-socket] [--idle-lock <minutes>] [--ask-timeout <seconds>]
rowel serve --install-host | --uninstall-host [--data-dir <path>]
rowel approve [<id> [--item <id>] [--days N | --run] [--name <name>] [--account <n>] [--deny]] [--data-dir <path>]
rowel admin rules list|add|remove … | agents on|off [--lockbox <id>] | key-file <path> | lock
```

`serve` unlocks a device-held lockbox from the terminal or a key file and
serves agents over the socket and a WebSocket under standing rules from a
config file; `approve` answers the asks it queues; `admin` edits the rules,
a lockbox's agent access and the key file. All of it is in
[`serve.md`](serve.md).

A Rowel Server is not run or administered from this binary: it is a separate
product with its own. This one is only ever its client, as an agent or a
sidecar that proves its workload identity (below), over the API in
[`server-api.md`](server-api.md).

## `rowel status`

One line per fact: whether the app is reachable, whether the vault is open,
and for each of the two keys where it lives (`in the login keychain`, `in
Credential Manager`, or the file's path), whether it exists and whether the
app has let it in. Exits 0 when the app is reachable, 1 when it is not. It
changes nothing — a key the app does not know yet is reported, not associated.

## `rowel mcp`

A stdio [MCP](https://modelcontextprotocol.io) server for a coding agent.
Five tools, each a 1:1 mapping onto an agent action of the broker protocol
(`spec/04-protocol.md`). No tool ever returns a value: a sign-in is filled on
the page by the browser extension; a secret for a command or an API goes to
the sidecar.

| tool | asks for | returns |
| --- | --- | --- |
| `rowel_request_login { site, reason? }` | to sign in to `site` with one of the user's logins (`site` is an origin or a URL; only its origin is sent); the call waits for the user's answer | the grant: `grantId`, `state` (`active`, `pending`, `denied`), `expiresAtMs`, `retryAfterMs` when pending, `items` as `{ id, title, username }` |
| `rowel_request_secret { ref, mode, target, reason? }` | to use the secret `ref` names (a placeholder) in `mode` `env` — `target` is the full command line, exactly as it will run — or `header` — `target` is `[METHOD[,METHOD] ]https://host/path-glob`, e.g. `GET,POST https://api.stripe.com/v1/*`, never a query string; the call waits for the user's answer | the grant as above, plus `use`: the one line that uses it — for env, `rowel run --grant <id> --env NAME=<ref> -- <command>`; for header, the `X-Rowel-Grant: <id>` header and the placeholder to send through `rowel proxy` |
| `rowel_wait { grant_id }` | the state of a grant left `pending` without a `retryAfterMs` (an approval routed to someone else); the desktop app answers at once, a server holds the call while the approval is routed; call again while it is still pending | the grant as it stands |
| `rowel_list_visible {}` | what the user has made visible to this agent | `items` as `{ ref, title, kind, fields }` |
| `rowel_end { grant_id }` | to end a grant; a sign-in grant also signs the browser out of the site | `{ ok: true }` |

`rowel_request_secret` is the agent's half of `env` and `header` mode. The
request names the placeholder's item and the one target the agent will use it
at; the grant is bound to that target, so a command line other than the one
asked for, or a request to another host, path or method, resolves nothing.
The agent gets the grant id and the line to run; the value goes to the sidecar
that runs it — `rowel run` for env, `rowel proxy` for header.

A `pending` *with* `retryAfterMs` means Rowel's one consent dialog was busy
with something else (another extension's ask, a passkey) and the request was
never put to the user, so no grant was issued. The server asks again on the
agent's behalf after that delay, for up to a minute; a `pending` the agent
still gets back is one to request again, not one to wait on.

A refusal is a tool error (`isError: true`) whose text is `{ error, message }`
— `error` is the broker's code name (`AgentsDisabled`, `GrantDenied`,
`DatabaseNotOpened`, …) or `unreachable` when the app is not running.
`AuditUnavailable` means the app could not write the audit row the release
stood on, so it released nothing; it is the host's trouble, not the grant's,
and the same request may succeed once the log takes rows again.

`initialize` and `tools/list` work with the app closed; a tool call needs it
running and the vault unlocked.

`rowel mcp` tells the app what launched it: `--platform claude-code|codex|cursor|…`
(default: `$ROWEL_PLATFORM`, else detected from the environment the agent
launches it in, else `cli`) and `--name` for the name it introduces itself as
until the dialog gives it one (default: the platform's label).

## `rowel run`

```
rowel run [--grant <id>] [--env NAME=rowel://…]… [--env-file <path>] [--reason <text>] -- <cmd> [args…]
```

Runs one command with placeholders resolved into *its* environment and
nowhere else. The parent shell — and so the agent that typed the command —
never holds the value.

The variables to set come from three places, in order of precedence: `--env`
flags, then `NAME=rowel://…` lines of `--env-file` (blank lines and `#`
comments skipped, an `export ` prefix accepted), then every variable of the
inherited environment whose value starts with `rowel://`. A value that is not
a well-formed placeholder where one is expected is a usage error (exit 2):
you meant a secret, and would otherwise get the misspelling.

**The command line.** A grant in `env` mode names the *full* command line
(`spec/03`), so `rowel run` and everything that talks about its command must
write it the same way. The rule: the words after `--`, joined by one space,
each word bare unless it is empty or contains whitespace, a quote (`'` or
`"`), `$`, a backtick, `\`, `*`, `?`, `|`, `&`, `;`, `<`, `>`, `(` or `)` — in
which case it is in single quotes, with an embedded `'` written `'\''`. So
`sh -c 'echo $KEY'` is the line for the words `sh`, `-c`, `echo $KEY`, and it
is what the grant names, what every resolve is checked against, what the hook
writes, and what `rowel_request_secret`'s `target` has to be.

With no placeholder at all the command simply runs; nothing is asked. Without
`--grant`, `rowel run` asks for one itself, under the sidecar key: the
placeholders' items, this command line as the target, mode `env`, and your
`--reason`. Rowel asks you (env mode is never silent unless a standing rule
names the full command line); a `denied`, or a refusal, is one line on stderr
and exit 1. The grant is for this one command, so it is ended as soon as
every value is resolved, before the command starts: nothing after that needs
it, and a signal or a kill cannot leave it open. A `--grant` you bring — from `rowel_request_secret`, say — is
used and left alone.

Every placeholder is then resolved under the grant at this command line; one
the grant does not cover is named on stderr (never its value) and exit 1. The
child runs with those variables set and the rest of the environment inherited,
stdin passed through, stdout and stderr piped through the redactor (below) to
yours, and `rowel run` exits with the child's exit code — or 1 when a signal
ended it.

Exit codes: 2 for a usage mistake, 1 for denied, unreachable, not resolvable
or a child a signal ended, otherwise the child's.

## `rowel proxy`

```
rowel proxy [--listen 127.0.0.1:7400] [--allow-http]
```

`header` mode: a local HTTP proxy that puts a secret into one request. The
agent calls an API through it with a placeholder where the key would go and
the grant's id in a header; the proxy resolves the placeholder under that
grant, swaps it into the outgoing request, sends it over TLS and redacts the
value from what comes back. The agent's process, transcript and environment
never contain the key.

Run `rowel proxy` (it keeps running; the first request creates and lets in
the sidecar key, as `rowel run` does), then point the SDK's base URL at it:

```
GET http://127.0.0.1:7400/https/api.stripe.com/v1/balance
Authorization: Bearer rowel://work/stripe/apiKey
X-Rowel-Grant: <grant id from rowel_request_secret>
```

- **Addressing**: `/https/<host>/<path>?<query>` is sent to
  `https://<host>/<path>?<query>`; `<host>` may carry a port. `/http/<host>/…`
  is served only with `--allow-http` (a dev server on localhost): a key sent
  in the clear is a key given away. Path-prefix addressing because every SDK
  takes a base URL, which is all the agent needs; a `CONNECT` forward proxy
  with a local CA is not offered (it would mean installing a CA in the
  agent's trust store).
- **The grant**: `X-Rowel-Grant` is required. Every `rowel://…` placeholder
  in a request header value, and in the body when `Content-Type` is
  `application/json` or `application/x-www-form-urlencoded` (there also in
  its percent-encoded form, `rowel%3A%2F%2F…`), is resolved under that grant
  for this host, path (without the query) and method — each distinct
  placeholder once per request, every request a use — and replaced in place.
  A placeholder anywhere in the URL is refused: URLs land in logs upstream.
  A request with no placeholder to put in is refused too (`no_placeholder`),
  never forwarded: the resolve is what checks the grant covers the host,
  path and method, so the proxy forwards only what a live grant covers.
  Call an API that needs no secret directly.
- **The host**, everywhere an HTTP target names one (the grant
  `rowel_request_secret` asks for, every resolve, `rowel redact --http`): lower
  case, with the port when it is not the scheme's default (`localhost:3000`
  stays, `api.example.com:443` is `api.example.com` under https), IPv6
  brackets kept. One rule, so a grant for a dev server on a port matches the
  requests it was asked for.
- **How a value is written** depends on where it lands, decided once per
  request from the content type: as is in a header; JSON-escaped in a JSON
  body (a placeholder in a JSON document is always inside a string, and the
  string becomes the one you wrote with the value in it); form-encoded in a
  form body, whether you wrote the placeholder plain or percent-encoded. A
  value holding a quote, a backslash, `&` or `=` therefore cannot
  restructure the document or add a field.
- **Redirects are not followed.** The grant was checked for the host, path
  and method you asked for, not for wherever the upstream points next, so a
  3xx comes back to you — status and body, redacted like any response — with
  its `Location` addressed through the proxy: `/other` or
  `https://elsewhere.example/x` comes back as `/https/<host>/other` or
  `/https/elsewhere.example/x`, a path on the proxy. Your client follows it
  (or you rebuild the hop) back through the proxy, where it is a request of
  its own with its own grant check: a hop to a host the grant does not name
  is refused there, and nothing ever goes to it directly. A hop that carries
  no placeholder (a client that drops `Authorization` on a redirect) is
  refused as `no_placeholder`; follow it directly or put the placeholder back. A `Location` that
  is not an `http`/`https` URL is dropped.
- **Forwarding**: the method and headers are kept; `X-Rowel-Grant`, `Host`,
  `Content-Length` and the hop-by-hop headers (`Connection`, `Keep-Alive`,
  `Proxy-*`, `TE`, `Trailer`, `Transfer-Encoding`, `Upgrade`) are dropped.
  The body is buffered whole, at most 32 MiB.
- **The response** is asked for unencoded (`Accept-Encoding: identity`,
  whatever your client sent), since a compressed echo of a value would pass
  the redactor; an upstream that answers encoded anyway gets a `502`. Status,
  headers and body come back with every value this request released redacted (its placeholder in its place, in headers and
  body alike, through the same redactor as `rowel run`); hop-by-hop headers
  are dropped and `Content-Length` is the redacted body's.
- **Limits**: 60 requests a minute per grant (a bucket of 60, refilled one
  a second), so a looping agent cannot hammer an upstream with a real key. A
  bucket is kept only for a grant the app has accepted, from the first
  request whose placeholders resolved under it: a made-up id is refused and
  takes no memory.
- **Logging**: one line on stderr per request — method, `/https/<host>/<path>`
  and the status; never the query, a header or a body.
- **Listening**: the loopback address only; a `--listen` on any other address
  is a usage error. The proxy holds the sidecar key and must not be reachable
  from the network.

Refusals are JSON, `{ "error": "<word>", "message": "…" }`, naming the
placeholder when one is at fault and never a value:

| status | `error` | when |
| --- | --- | --- |
| 400 | `bad_request` | a path not of the form `/https/<host>/<path>`, `/http/` without `--allow-http`, a placeholder in the URL |
| 400 | `no_placeholder` | nothing in the headers or the JSON or form body to put a value in |
| 400 | `bad_header` | a resolved value that cannot be sent as a header |
| 403 | `no_grant` | no `X-Rowel-Grant` |
| 403 | the broker's code name (`GrantDenied`, `GrantExpired`, …) or `unreachable` | a placeholder the grant does not cover, or the app not running |
| 503 | `AuditUnavailable` | the app could not write the `used` row, so it released nothing |
| 413 | `body_too_large` | a body over 32 MiB |
| 429 | `rate_limited` (with `Retry-After`) | the bucket is empty |
| 502 | `upstream` | the upstream could not be reached or did not answer |

## `rowel redact`

```
rowel redact [--grant <id> --ref <rowel://…>… (--command <line> | --http <METHOD> <host/path>)]
```

The redactor as a filter: stdin to stdout. Its values come the only
legitimate way, by resolving each `--ref` under the grant at the place the
grant names (`--command` for an env grant, `--http GET api.stripe.com/v1/balance`
for a header grant). With no `--ref` it copies.

The redactor replaces every occurrence of a released value with its
placeholder, in the value's exact bytes and in the forms a program commonly
prints it: standard and URL-safe base64 (padded or not), percent-encoded
(RFC 3986, upper-case hex) and JSON-escaped. It streams at a constant lag:
the last bytes it has read, one fewer than the longest form, are always held
until more output comes or the stream ends, whatever they are, so a value
split across two reads is still caught and how soon output appears never
says whether it began like a value. Output therefore trails by up to that
many bytes (a prompt the program prints may wait for the next line or for
the end), and there is no flush on a timer: one would print a slowly printed
value in pieces. It is a safety net, not a guarantee: a value
hashed, truncated, printed in parts, or transformed any other way passes.
**Values shorter than 6 bytes are not matched at all** — a needle that short
would redact ordinary text.

## `rowel hook`

```
rowel hook pre|post [--platform claude-code|codex|cursor]
```

A coding agent's tool hooks: one event read as JSON on stdin, the hook's
answer written on stdout, nothing when there is nothing to say. The platform
is detected from the environment the agent runs the hook in (`$CLAUDECODE`,
`$CURSOR_TRACE_ID`, `$CODEX_SANDBOX`, or `$ROWEL_PLATFORM`); `--platform`
says it outright.

- **`pre`** — a shell command of the form `NAME=rowel://… [NAME2=rowel://…] cmd args…`
  (one plain command: no `|`, `&&`, `||`, `;`, newline, redirection,
  backtick, `$(` or `${`) is rewritten to
  `rowel run --env NAME=rowel://… -- cmd args…`, with the arguments quoted
  by the rule above, so the agent's own shell never has the value. Anything
  else is left untouched. `rowel` is written as the hook was invoked, so the
  rewritten command finds the same binary.
- **`post`** — the CLI's exposure check, the counterpart of the extension's.
  For a `rowel run --grant <id> --env NAME=rowel://… -- …` command, the
  placeholders are resolved under that grant at that command line and the
  tool's output scanned for the values. A hit ends the grant (a refusal is
  ignored: the grant may be an agent's, which only it can end) and adds a
  line of context for the agent saying a secret released under grant `<id>`
  appeared in the output and the grant was ended — never the value. No
  grant in the command (`rowel run` asked for and ended its own), no
  placeholders, or no hit: nothing. The file of `--env-file` is not read by
  the hook.

What each platform's hooks can do, from their documentation:

| platform | event | can rewrite the command | can add context after |
| --- | --- | --- | --- |
| Claude Code | `PreToolUse` / `PostToolUse` on `Bash` ([docs](https://code.claude.com/docs/en/hooks)) | yes: `hookSpecificOutput.updatedInput` replaces the whole `tool_input` | yes: `hookSpecificOutput.additionalContext` |
| Codex CLI | `PreToolUse` / `PostToolUse` on `Bash` ([docs](https://developers.openai.com/codex/hooks)) | yes: `updatedInput` beside `permissionDecision: "allow"`, which Codex requires | yes: `hookSpecificOutput.additionalContext` |
| Cursor | `preToolUse` / `postToolUse` on `Shell` ([docs](https://cursor.com/docs/agent/hooks)) | yes: `updated_input` beside `permission: "allow"`; `beforeShellExecution` cannot | yes: `additional_context` |

The hook reads `tool_input.command` on every platform; the output it scans is
Claude Code's `tool_response` (an object with `stdout` and `stderr`), Codex's
`tool_response` (tool-specific; its docs do not fix Bash's shape) and
Cursor's `tool_output` (a JSON string), each scanned as text so a value in
any field is seen. On Claude Code the rewrite carries no permission
decision: the rewritten `rowel run` goes through the platform's own permission
rules as the original would have, and Rowel's dialog gates the secret. On
Codex and Cursor the platform applies a rewrite only beside an `allow`, so
there the dialog is the gate. A platform whose hook cannot rewrite a command
gets the MCP tool plus `rowel run` by hand: the agent calls
`rowel_request_secret` and runs the `use` line.

Claude Code, in `~/.claude/settings.json` or the project's `.claude/settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [{ "type": "command", "command": "rowel hook pre" }] }
    ],
    "PostToolUse": [
      { "matcher": "Bash", "hooks": [{ "type": "command", "command": "rowel hook post" }] }
    ]
  }
}
```

Codex, in `~/.codex/hooks.json` (or the project's `.codex/hooks.json`), the
same shape:

```json
{
  "hooks": {
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [{ "type": "command", "command": "rowel hook pre" }] }
    ],
    "PostToolUse": [
      { "matcher": "Bash", "hooks": [{ "type": "command", "command": "rowel hook post" }] }
    ]
  }
}
```

Cursor, in `.cursor/hooks.json` (project) or `~/.cursor/hooks.json`:

```json
{
  "version": 1,
  "hooks": {
    "preToolUse": [{ "command": "rowel hook pre --platform cursor", "matcher": "Shell" }],
    "postToolUse": [{ "command": "rowel hook post --platform cursor", "matcher": "Shell" }]
  }
}
```

### Connecting an agent

Two switches in Rowel first: Settings › Browser & agents › **Agents** must be
on, and the lockbox the agent should see must have its agent access on. With
Agents off, every tool call answers *Turn on Agents in Rowel › Settings ›
Browser & agents*.

A login is filled by the Rowel browser extension, so for `rowel_request_login`
the agent must drive a browser profile that has the extension installed and
connected to this app. A fresh profile an automation tool launches has none:
install the extension in the profile the agent uses and let it connect first.
When the grant ends, session end signs that profile out of the site's domain.

Claude Code (here `/usr/local/bin/rowel` stands for the path Settings ›
General › Command line tool shows, written out in full — these files do not
expand `~`):

```bash
claude mcp add rowel -- /usr/local/bin/rowel mcp
```

Codex, in `~/.codex/config.toml`:

```toml
[mcp_servers.rowel]
command = "/usr/local/bin/rowel"
args = ["mcp"]
```

Cursor, in `.cursor/mcp.json` (project) or `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "rowel": { "command": "/usr/local/bin/rowel", "args": ["mcp"] }
  }
}
```

A bare `rowel` works only for an agent started from a terminal whose `PATH`
has it; see [Installing](#installing). A Windows path needs each backslash
doubled in JSON and in a double-quoted TOML string, or forward slashes in its
place.
