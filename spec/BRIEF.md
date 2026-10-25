# Cold-start brief: Rowel for Agents

Read this first. It gets an agent with no prior context to the point where it
can pick any step of `PLAN.md` and work it to the project's standard. Ten
minutes of reading; everything else is linked.

## 1. What Rowel is, and what we are building

Rowel is a free, open-source, local-first secrets manager: a Tauri 2 desktop
app (Rust core, React frontend), an iOS app, and a browser extension forked
from KeePassXC-Browser. One master password opens a vault of *lockboxes*;
each lockbox is a SQLCipher file with every item sealed again inside it; sync
goes through the user's own cloud as an opaque pack; there is no account and
no backend. Read `README.md` and `docs/threat-model.md` ("Architecture in one
paragraph" and "The browser extension") for the shape of the product.

We are adding **agents**: AI coding agents and agentic browsers get to *use*
credentials without ever *holding* them. The agent asks; a person or a rule
says yes; Rowel fills the form, signs the passkey, swaps the header at the
proxy, or hands the value to one child process; everything is logged. Three
tiers run one engine: the desktop app as broker (free), a headless server mode
of the CLI (companies), and Rowel Cloud on Cloudflare (hosted). The business
direction: free for personal and family use, paid for companies' agents.

## 2. Where the knowledge is

All of it lives in this `spec/` directory, which is **kept out of the main
branches on purpose** (see §4 for how to get it).

| Read | For |
| --- | --- |
| `SUMMARY.md` | The plain-language version: what, why, the steps, the open decisions |
| `README.md` | The index of the spec and its revision history |
| `00-overview.md` | The three tiers, the reuse map, the repository and license map, the glossary |
| `01`–`09` | The design, one concern each; every section has a *Technical details* part naming real files and types |
| `10-open-questions.md` | Everything not settled, Q1–Q26, with the assumption the spec runs on |
| `PLAN.md` | **The checklist.** Eleven iterations; tick items as they land |
| `11-roadmap.md` | Phase 0's PR-level acceptance criteria that PLAN's I0–I4 reference |

The project context (Fletch's `context_get`) holds the recorded decisions;
`spec/README.md`'s revision history lists what changed and why.

## 3. Where we are

- **I0, I1, I2**: merged into `main` (PRs #652, #653, #655).
- **I3, secrets for commands and APIs (the sidecar)**: PR #657 (`feat/agent-sidecar`, on `main`), in review.
- **I4 onward**: not started. `PLAN.md`'s headers say which iteration is in progress and where.

Check `PLAN.md` first; its headers carry the current PR numbers and branches.
Then run `pr_list`/`pr_checks` through the Fletch RPC mailbox for the live state.

## 4. How to get the spec when you start cold

The spec lives on its own branch, `spec/rowel-agents`, and is never merged
into `main`. To work with it from any code branch without committing it:

```sh
git fetch origin spec/rowel-agents        # or the git_fetch RPC op with ref "spec/rowel-agents"
git checkout origin/spec/rowel-agents -- spec/
git reset -q -- spec/                    # leave it untracked: never stage spec/
```

Rules: stage files by explicit path, never `git add -A`; if you change the
spec (you should, when the code settles on something different), commit that
change **on the spec branch only** and push it there; code branches never
carry `spec/`.

## 5. How a step is worked

1. Read the iteration in `PLAN.md`, then every spec section it references.
   Settle its *Decide first* questions with the owner before starting if they
   are owner decisions (Q1–Q4, Q24).
2. Branch from the right base: iterations stack. I1 sits on I0's branch; I2
   sits on I1's. Name branches `feat/…`, `refactor/…`, `chore/…`.
3. Slice the iteration into reviewable pieces with disjoint file ownership
   (Rust foundation → app integration ∥ extension, as I1 was done), and give
   each implementing agent a brief that names the spec sections, the files it
   owns, the exact API it builds on, the gates, and the report format.
4. Review each piece yourself against the spec before the next depends on it:
   public API shapes, lock order and session/epoch handling, everything on
   the wire, test files. Send corrections back to the same agent.
5. Run the full gates on the combined tree, commit with `git commit -s`
   (DCO sign-off is required), one commit per slice, and open the PR with the
   `open_pr` RPC op, `args.base` naming the branch below in the stack, and a
   body that ends with the `## Decisions` section in the required shape.
6. Update the spec for anything built differently, record decisions in the
   project context, tick the PLAN items when the PR merges.

## 6. Gates and pitfalls (learned the hard way)

- Rust: `cargo fmt --all -- --check`; `rtk proxy cargo clippy --all-targets -- -D warnings`
  (the shell proxy can drop the `--`; `rtk proxy` runs the raw command);
  `cargo test`; for iOS `cargo clippy -p rowel --target aarch64-apple-ios-sim --all-targets -- -D warnings`
  (CI runs it; desktop-only code must be `#[cfg(desktop)]`-gated).
- Frontend: `bun run typecheck`, `bun run lint` (0 warnings), `bun run test`.
  Vitest needs Node 24: `export PATH="$HOME/.nvm/versions/node/v24.14.0/bin:$PATH"`.
- Extension: `bun run extension:lint`, `bun run extension:build`,
  `cd extension && npx playwright test --project=chromium`. Firefox cannot
  launch in the Fletch sandbox (pre-existing); CI covers it.
- The Bash tool's working directory persists across parallel calls: use
  absolute paths in every command, or `cd` at the start of each.
- The sandbox refuses writes to `.git/config` and `.git/info/exclude`
  (branch rename partly fails; harmless) but commits, refs and pushes work.
- Comments in this codebase explain *why*, in full sentences, and are part of
  the design record; keep the voice. No narrating comments, no dead code.
- Never log or put in an audit row a secret, a TOTP code, or a URL's query.

## 7. The prompt that starts a step

> Fetch the `spec/rowel-agents` branch and copy its `spec/` into the working
> tree untracked. Read `spec/BRIEF.md`, then `spec/PLAN.md`, then
> `spec/00-overview.md`. Work iteration **I\<n\>** of `PLAN.md` exactly as
> §5 of the brief describes: read every spec section the iteration references,
> slice it, brief implementing agents, review each slice against the spec,
> run every gate, commit with sign-off, open a stacked PR with a Decisions
> section, update the spec on the spec branch for anything built differently.
