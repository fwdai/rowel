# The command line: `rowel-cli`

Rowel ships a small command line beside the app, `rowel-cli` (the crate is
`src-tauri/crates/cli`). It talks to the running app over the same socket and
sealed channel the browser extension uses, as a client of kind `agent`: it can
ask for grants and nothing else. On macOS and Linux, Settings › General ›
Command line tool installs a `rowel` symlink to it on your `PATH`; on Windows
the row shows the bundled path to add yourself. Below, `rowel` stands for
whichever name you have.

## `rowel status`

One line per fact: whether the app is reachable, whether the vault is open,
where this machine's agent key is and whether it exists, and whether the app
knows it. Exits 0 when the app is reachable, 1 when it is not. It changes
nothing — a key the app does not know yet is reported, not associated.

## `rowel mcp`

A stdio [MCP](https://modelcontextprotocol.io) server for a coding agent.
Five tools, each a 1:1 mapping onto an agent action of the broker protocol
(`spec/04-protocol.md`). No tool ever returns a value: a sign-in is filled on
the page by the browser extension, and a secret is served to the request or
the command by Rowel.

| tool | asks for | returns |
| --- | --- | --- |
| `rowel_request_login { site, reason? }` | to sign in to `site` with one of the user's logins (`site` is an origin or a URL; only its origin is sent) | the grant: `grantId`, `state` (`active`, `pending`, `denied`), `expiresAtMs`, `retryAfterMs` when pending, `items` as `{ id, title, username }` |
| `rowel_request_secret { ref, mode, target, reason? }` | to use the item `ref` names (`rowel://<lockbox>/<item>/<field>`) as a header on calls to `target` (`mode: "header"`, `target` is `host/path-glob`) or as an environment variable of `target` (`mode: "env"`, the full command line) | the grant, as above |
| `rowel_wait { grant_id }` | to wait up to a minute for the user to answer a pending grant | the grant as it stands |
| `rowel_list_visible {}` | what the user has made visible to this agent | `items` as `{ ref, title, kind, fields }` |
| `rowel_end { grant_id }` | to end a grant; a sign-in grant also signs the browser out of the site | `{ ok: true }` |

A refusal is a tool error (`isError: true`) whose text is `{ error, message }`
— `error` is the broker's code name (`AgentsDisabled`, `GrantDenied`,
`DatabaseNotOpened`, …) or `unreachable` when the app is not running.

The first tool call creates this machine's agent key at
`<config dir>/rowel/agent.json` (`~/Library/Application Support/rowel/agent.json`
on macOS, `~/.config/rowel/agent.json` on Linux, `%APPDATA%\rowel\agent.json`
on Windows; the file holds the public key and the name the app gave the agent)
and asks the app to let it in — Rowel shows the dialog and you give the agent a
name. `initialize` and `tools/list` work with the app closed; a tool call needs
it running and the vault unlocked.

`rowel mcp` tells the app what launched it: `--platform claude-code|codex|cursor|…`
(default: `$ROWEL_PLATFORM`, else detected from the environment the agent
launches it in, else `cli`) and `--name` for the name it introduces itself as
until the dialog gives it one (default: the platform's label).

### Connecting an agent

Two switches in Rowel first: Settings › Browser & agents › **Agents** must be
on, and the lockbox the agent should see must have its agent access on. With
Agents off, every tool call answers *Turn on Agents in Rowel › Settings ›
Browser & agents*.

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

On Windows, or without the symlink, replace `rowel` with the full path
Settings › General › Command line tool shows.
