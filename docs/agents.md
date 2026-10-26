# Agents

Rowel lets an AI agent use your logins and API keys without ever holding them.
The agent asks for one item at one place; you approve it in Rowel's dialog, once
or for a number of days; Rowel uses the credential where it is needed, by
filling the login in the browser or putting the key into one command or one
HTTP request; and every decision and use is written to a log you can read in
Vault Health. The agent gets back a grant id and an outcome, never the value,
so a prompt injection, a leaked transcript or an `env` dump yields placeholders.

This works with coding agents (Claude Code, Codex, Cursor) through the `rowel`
command line tool, and with agentic browsers (Claude in Chrome, Comet, a
Browser Use script) through the Rowel browser extension. Everything runs on
your machine against your own vault; there is no server in the loop.

## Turning it on

Two switches, both off by default:

1. **A lockbox for agents.** Make a lockbox for the items agents may use, the
   way you would split Personal from Work, and put the logins and API keys
   they need in it. In that lockbox's settings turn on **Agents can use items
   in this lockbox**. A lockbox with this off never answers an agent, whatever
   you approve elsewhere.
2. **The Agents switch.** In Settings › Browser & agents turn on **Agents**.
   With it off, every agent request is refused and the agent is told to turn
   it on.

Agents are served only while Rowel is unlocked and the lockbox they use is the
open one. Locking Rowel ends every grant.

## Agentic browsers

For a browser profile an agent drives, open the Rowel extension's popup and
turn on **Agent mode**. The popup then shows one screen, "Agent mode is on",
with a **Turn off** button, and the extension stops drawing its icons and
menus in pages, so the agent cannot click through the extension itself.

While agent mode is on, every fill is a grant. When the agent reaches a login
page, Rowel comes forward with a dialog that names the site, the login it would
fill and, when an agent asked for it, the agent and its reason. You answer:

- **Allow once**: fills this login on this site now, and for the rest of the
  same sign-in within the grant's 15 minutes (the TOTP step, a username-first
  second page, a retry).
- **Allow for N days**: the same, and writes a standing rule for this client,
  this login and this site, so the next visit fills without asking until the
  rule ends. The default N is set in Settings › Browser & agents (**Default
  length of a standing rule**, 1 to 90 days).
- **Deny**: fills nothing.

Cards and identities ask for a second check on top, Touch ID, Face ID or
Windows Hello, or your master password where there are no biometrics.
Passkeys always ask, agent or not: your tap is the user-presence gesture the
site requires.

After a fill, the extension checks that the page did not end up showing the
password or the code as text (a login form that echoes the password on error,
a debug page). If it did, the fields are emptied, the grant ends and you see
a notification. The check reads text, not pixels.

Standing rules, active grants and connected agents are listed in Settings ›
Browser & agents: **Rules** with **Remove**, **Active grants** with **End**, and
**Connected extensions and agents** with **Forget**. Turning the Agents switch
off, or a lockbox's agent access off, ends every active grant.

## Coding agents

Install the command line tool from Settings › General › **Command line tool**.
On macOS and Linux **Install** links it onto your `PATH` as `rowel`; on Windows
the row shows the bundled path, which you use in place of `rowel` below.

`rowel mcp` is a stdio MCP server. Connect it to your agent:

Claude Code:

```bash
claude mcp add rowel -- rowel mcp
```

Codex, in `~/.codex/config.toml`:

```toml
[mcp_servers.rowel]
command = "rowel"
args = ["mcp"]
```

Cursor, in `.cursor/mcp.json` (project) or `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "rowel": { "command": "rowel", "args": ["mcp"] }
  }
}
```

The first tool call asks Rowel to let the agent in, as a browser extension
does: Rowel shows a dialog naming the kind of client and the platform, and you
give it a name. The tools it gets are listed in
[`docs/cli.md`](./cli.md#rowel-mcp); none of them returns a value.

**Logging into a site.** The agent calls `rowel_request_login` for a site.
Rowel asks you in the same dialog as above, with the agent's reason. On
approval the Rowel browser extension fills the login in the open tab at that
site, so the agent must drive a browser profile that has the extension
installed and connected to this Rowel. The dialog tells you that when the
grant ends, Rowel signs that browser out of the site: on expiry (15 minutes),
when the agent calls `rowel_end`, when you end it in Settings, or when the page
exposed the secret, the extension deletes that site's cookies in the profile
that filled it. Safari's extension has no cookies API, so there you sign out
yourself.

## API keys and `.env` values

An agent names a secret by a placeholder and never sees it:

```
rowel://<lockbox>/<item>/<field>
```

for example `rowel://work/stripe/apiKey` or `rowel://work/backend-env/body.DATABASE_URL`.
The parts are the lockbox's name, the item's title and the field, each as a
slug; the exact rules and the field names per kind are in
[`docs/cli.md`](./cli.md#placeholders).

- **For one command**: `rowel run --env STRIPE_KEY=rowel://work/stripe/apiKey -- stripe balance retrieve`
  asks you to allow that exact command line, runs it with the value in its
  environment only, and passes its output through a redactor that replaces
  the value with the placeholder. The agent's shell never has the value. See
  [`rowel run`](./cli.md#rowel-run).
- **For an API**: run `rowel proxy` and point the SDK's base URL at
  `http://127.0.0.1:7400/https/api.stripe.com`. The agent sends the
  placeholder where the key would go and the grant id in an `X-Rowel-Grant`
  header; the proxy swaps in the value, forwards over TLS and scrubs it from
  the response. See [`rowel proxy`](./cli.md#rowel-proxy).
- **Hooks**: with `rowel hook pre` and `rowel hook post` configured as the
  agent's shell hooks, a command the agent writes as `STRIPE_KEY=rowel://… cmd`
  is rewritten into the `rowel run` form before it runs, and a command whose
  output shows a released value ends its grant. The configuration for each
  platform is in [`rowel hook`](./cli.md#rowel-hook).

Either way the dialog shows the exact place the secret goes: the full command
line, or the host, path and methods. **Allow for N days** writes a rule for
exactly that place. A command in `env` mode always asks unless a rule names its
full command line, because the command itself can print what it is given.

## Org lockboxes and a Rowel Server

A company that runs a Rowel Server can hand you lockboxes it holds, so that
its agents can use them while nobody is at a keyboard and you can approve
what they ask for from your own Rowel. The app's side of that is in Settings
› Lockboxes › **Rowel Server**:

- **Connect.** Enter the server's address and sign in; the browser opens on
  the server's sign-in page, which is your company's, and comes back to Rowel.
  Rowel enrolls this computer as one of your devices under its host name.
- **A pending device.** A new device is *pending* until one of your already
  approved devices, or an admin, approves it. Each device shows a short
  fingerprint of its enrollment key; compare the one on the new device's
  screen with the one in the list before you tap **Approve**. Until then the
  server hands the device no keys, so a stolen sign-in alone opens nothing.
- **Add.** Once approved, the server lists the org lockboxes you may hold;
  **Add** puts one beside your other lockboxes, synced through the server
  rather than through your cloud account. It opens with your master password
  like the rest, has its own agent-access switch, and cannot be renamed or
  re-keyed from here: its name and its key are the server's. If the server
  rotates a lockbox's key, your copy follows at its next sync.
- **Approvals.** While Rowel is unlocked, a request an agent makes at the
  server for one of your lockboxes reaches you in the same dialog as a local
  agent's, with the company named. Allow once, for N days or deny, as before;
  cards and identities take the same second check. Your answer goes back to
  the server, which keeps the log for its own grants; nothing is written to
  your Vault Health for them.

What is different about an org lockbox, in one line: the server can open it.
Its key is not derived from your password; the server minted it and holds a
copy under its own root key, which is what lets it serve an agent at 3 a.m.
Rowel says so when you add one. Everything else is unchanged: a client of
kind `agent` still never receives a value, in any tier.

## What you will see in Vault Health

Vault Health has an **Agent activity** panel:

- the last seven days of uses, denials (with the reason: no rule allowed it,
  you refused, the ask timed out, and so on) and exposures, one row per event;
- standing rules older than 30 days, so a "for 7 days" you kept renewing does
  not become permanent without your noticing;
- agents that have not been seen in 90 days, with **Forget** beside each;
- **Verify the log**, which walks every device's chain of rows and names the
  first one that does not hold: a missing row, one that does not follow the
  row before it, or one that was altered.

A row says who did what with which item, where, and how it was decided. Read
together, the rows of one sign-in look like this:

```
09:14:03  Claude Code (run 7f3a…) asked to fill the github.com login alex@…
          policy: ask
09:14:09  allowed by you (Touch ID)
09:14:10  filled github.com, submitted, nothing exposed
09:29:10  grant ended (expired), signed out of github.com
```

A row never holds a password, a code or a URL's query string. The log lives
inside the lockbox, so it is encrypted with it, syncs with it and comes back
with a restore; two devices end up with the union of their rows. Rows older
than a year are pruned.

## What is and is not protected

The full account is in the threat model's [Agents](./threat-model.md#agents)
section. In short:

- The agent never receives a secret value, in any mode. What it holds is a
  grant id, which is good only for the item, place and mode you approved, and
  only until the grant ends.
- **`env` mode is the weakest.** The command you approve receives the real
  value and can print, store or send it. Approve only command lines you would
  type yourself, and read a "for N days" rule as "this exact command, without
  asking, until then".
- **The redactor is a safety net, not a guarantee.** It catches the value
  printed as is or in the common encodings (base64, percent-encoding, JSON
  escaping). A value hashed, truncated, split or shown as pixels passes, and
  values shorter than six bytes are never matched.
- **A logged-in session is a session.** Rowel signs the agent's browser out of
  the site when the grant ends. While the grant is live, an agent that controls
  the browser can use the session like any logged-in user.
- **Same-user processes.** Another program running as you can lift the
  extension's or the command line tool's keys and ask in their name; the
  dialog and the rules still decide what it gets. On macOS and Windows the
  keys live in the OS secure store, where a foreign read prompts you.

On a machine with no desktop, the same broker runs from the command line as
`rowel serve`, unlocked from a terminal or a key file, with standing rules in
a config file and asks answered by `rowel approve`; see
[`docs/serve.md`](./serve.md). A company runs the Rowel Server itself with
`rowel serve --server`; see [`docs/server.md`](./server.md). Not in this
release: connecting to a server from the phone, approvals pushed to a phone,
key rotation and offboarding at the server, OAuth token minting, and agents
in remote browsers.
