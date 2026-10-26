# Rowel for Agents, in plain language

This is the short version of the plan for people who do not need the
internals. The full design is in the other files in this folder, and the
step-by-step checklist is `PLAN.md`.

## What we are building, and why

AI agents are starting to do real work for people: log into websites, call
APIs, run commands. To do that they need passwords and keys. Today people hand
those over, either by pasting them into a chat or by putting them in a file on
the machine where the agent runs. That is dangerous in a new way. A normal
program does what its code says. An agent does what its inputs say, and its
inputs are web pages, emails and documents that anyone can write. In 2026 this
stopped being a theory: a single malicious line in a pull request made three
well-known coding agents publish their company's secrets.

Rowel's answer is simple to say: **the agent never gets the secret.** It asks
Rowel for permission to use one. A person says yes, or a rule the person wrote
earlier says yes. Then Rowel does the sensitive part itself: it fills the login
form, signs in with a passkey, puts the API key into the request on the way
out, or hands the key to one single command and nothing else. The agent only
learns that it worked. Every use is written down: which agent, for whom, which
account, where, approved by whom.

This builds on what Rowel already is. The desktop app already fills logins for
your browser and already has the dialog that asks you before something
sensitive happens. We extend that, rather than building a second system.

## Three products from one engine

1. **Free, on your own computer.** The Rowel app you already run becomes the
   gatekeeper for agents on that machine: coding assistants, browser agents,
   scripts. You approve on the spot with a tap or Touch ID, or set a rule like
   "let Claude Code use my GitHub login for a week". Nothing leaves your
   computer. This stays free and open source.

2. **A server your company runs.** Companies have agents running where nobody
   is sitting: in build pipelines, on servers, in cloud browsers. The server
   holds the company's shared lockboxes, recognises each agent by the identity
   its platform already gives it, applies the company's rules, and sends
   approval requests to the right person's Rowel app. Employees see a company
   lockbox beside their personal ones, behind the same master password.

3. **Rowel Cloud.** The same server, run by us, with sign-up, billing, phone
   notifications for approvals, single sign-on and exports to the tools
   security teams already use. Built on Cloudflare's infrastructure so we run
   as little of our own as possible.

The important technical fact is that all three run the **same engine**. The
free app, the self-hosted server and the cloud are one codebase with more
capabilities switched on at each level. We are not writing anything twice.

## One honest trade-off to understand

Rowel's promise so far has been "no server can ever read your secrets". For a
company's agents that run at 3 a.m. with nobody present, something other than
a person must be able to open the lockbox. So there are two kinds of lockbox:

- **Your own lockboxes** stay exactly as they are: only your master password
  opens them, no server can read them, ever.
- **Company lockboxes** live on the server, and the server can open them,
  because it has to act for agents while people sleep. The app says this in
  plain words when such a lockbox is created, and the company chooses what
  goes in it.

Separately, every lockbox has a switch, off by default, that says whether
agents may use anything in it at all.

## How we will build it: eleven steps

Each step produces something people can use. If we stop after any step, we
have a working product, just without the later features.

1. **Tidy the foundation.** Move the existing browser-extension machinery into
   a shared component. Nothing changes for users. *About two weeks of work.*
2. **Agent mode in the browser.** Turn it on, and any AI browser (Claude in
   Chrome, Comet and the like) must get your approval in Rowel for every login
   it fills. Rowel hides its own buttons from the agent and checks the page did
   not leak the password. *After this step we have the equivalent of the
   feature 1Password announced this week, free.*
3. **Coding agents can log in.** Claude Code, Codex and Cursor can ask Rowel
   to sign them into a site. You approve, the browser extension fills, and
   when the agent is done, Rowel signs that site out again.
4. **API keys and secrets for commands.** An agent can run one command or call
   one API with a key it never sees. Anything it prints that contains the key
   is blanked out before the agent reads it.
5. **A readable history.** Vault Health shows what your agents did, across
   your devices, with a tamper-evident log. The free product is complete and
   released.
6. **The engine without the app.** A command-line version of the same engine
   that runs on a developer's server or VM with no screen, approving from a
   terminal or by rules. This is the piece that becomes the company server.
7. **The app learns about company lockboxes.** The desktop and phone apps can
   hold a lockbox whose key comes from a server, keep it in sync, and answer
   that server's approval requests in the usual dialog.
8. **The company server, first version.** One program a company installs.
   Employees sign in with the company's identity provider. Agents in GitHub's
   build system prove who they are and ask for what they need; sensitive
   requests wait for a tap on the owner's Mac. Everything is exportable.
9. **The company server at scale.** Administration from inside the Rowel
   app, approval routing to teams, automatic key rotation when people leave,
   exports to security tools.
10. **Rowel Cloud, first version.** Sign up, pay, connect. The same server
    running on Cloudflare, with the company's data kept in the region they
    choose.
11. **Approvals on your phone, enterprise fit.** Approval requests arrive as
    notifications. Free users can approve a request from their laptop's agent
    on their phone through a relay that cannot read it. Single sign-on and
    automatic user provisioning for large customers.
12. **Reach.** Agents in cloud browsers, services that use OAuth tokens, more
    identity platforms, passkeys for company lockboxes once the standards
    settle, rules written as code.

Steps 1 to 5 are the free product. Steps 6 to 9 are the paid server. Steps 10
to 12 are the cloud and beyond.

## Decisions that are yours to make

These do not block the first steps, but they shape the paid product and should
be settled before step 8.

- **License for the server.** Keep everything open source and earn from
  hosting and support, or make the server source-available or proprietary
  while the app stays open. This decides who can run the server without
  paying. *(Settled: libraries permissive, apps GPL, server proprietary; for
  now the server is built in the public repository as its own crate, ready
  to move.)*
- **Self-hosted first, or cloud first.** The plan assumes self-hosted first,
  because the cloud is the same program plus operations, and early customers
  in regulated industries will want to run it inside.
- **Names.** What to call a company lockbox so that the word itself carries
  the warning that the server can open it, and what to call the per-lockbox
  "agents allowed" switch.
- **What to charge for.** The plan assumes a price per employee plus a
  metered price for agent activity above an allowance. It deliberately does
  not charge per lockbox or per agent, so nobody is tempted to cram everything
  into one.
- **An optional account in the free tier.** Approving a laptop agent from
  your phone needs a tiny account for notifications. Rowel's identity is "no
  account". The plan assumes we offer this as a clearly optional extra, used
  for nothing else.

## Decisions already made, so you know

- **Passkeys always ask.** An agent can sign in with a passkey from your
  vault, but your tap is always required. That tap is what makes a passkey a
  passkey; removing it would be cheating the standard.
- **The desktop app is not split in two.** Reuse happens in the shared code,
  not by running a background service under the app, which would put your
  keys in a second process for no security benefit.
- **One program for server and cloud.** The command-line engine from step 6
  is the server in step 8 and the thing that runs inside Rowel Cloud in
  step 10.
- **Cloudflare for the cloud,** with one exception: the engine itself runs in
  a container, because it needs real cryptography and memory it controls.
  Everything around it, routing, storage, coordination, notifications,
  single sign-on, is Cloudflare's.
- **The weakest mode is labelled as such.** Handing a key to a single command
  is still the riskiest way to use one, so it always asks unless a rule names
  the exact command.

## What this does not solve, honestly

- After Rowel logs an agent in, the agent is logged in. It can do what that
  account allows until Rowel signs it out at the end. Rowel protects the
  password and shortens the session; it does not supervise what the agent
  clicks.
- Some sites, banks especially, forbid automation in their terms and try to
  detect it. Rowel will warn, not pretend otherwise.
- Standards for agents and passkeys are still being written. We lead with the
  safe local version and wait on the server-side version.
- Anything a browser can show on screen can be photographed by an agent that
  uses vision. Rowel blanks text; it cannot blank pixels.

## Why a company would pay

Not for storage. Because the machine an agent runs on is now a hostile place
to keep a secret, and Rowel keeps secrets off it entirely. Because web logins,
API keys, passkeys and tokens are covered by one set of rules, where
everything else stops at API keys. Because every action is attributed to an
agent, a run and a person. Because revoking one agent does not mean rotating a
key everywhere. And because developers will already be using the free version
on their laptops, with the same placeholders that work in production.
