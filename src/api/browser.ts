import { call } from './client'

/**
 * The browser extension host (desktop only): Rowel speaks the KeePassXC-Browser
 * protocol, so its own extension, and the stock KeePassXC one, fill logins from
 * the vault. Every command that changes something answers with the whole
 * status as it now stands, so Settings redraws from what Rust says rather than
 * from a guess.
 */
export interface BrowserStatus {
  enabled: boolean
  /** Whether the host also registers under KeePassXC's name, for the stock extension. */
  keepassxcCompat: boolean
  /** Agents through the extension: the switch, and the grant dialog's default. */
  agents: AgentsSettings
  /**
   * Every browser Rust knows how to register with. `detected` is its profile
   * directory being there; `installed` is Rowel's own manifest, which is
   * written on enabling and only for a detected browser; `conflict` — only
   * ever set with `keepassxcCompat` on — is KeePassXC's name there already
   * held by KeePassXC itself, which Rowel leaves alone.
   */
  browsers: {
    id: string
    label: string
    detected: boolean
    installed: boolean
    conflict: boolean
  }[]
  /**
   * The clients let into the open vault — browser extensions and agents — by
   * the name they were given. Each vault keeps its own list.
   */
  clients: BrowserClient[]
}

/** What a client is, which decides what it may ask for: an extension fills, an agent asks for grants. */
export type ClientKind = 'extension' | 'agent' | 'sidecar'

/**
 * A client let into the open vault: the name the user gave it, its public
 * key, what it is, and — for an agent — what it said it runs on
 * (`claude-code`, `codex`, `cursor`, …). A row from before kinds reads as an
 * extension let in at time zero.
 */
export interface BrowserClient {
  name: string
  key: string
  kind: ClientKind
  platform?: string
  createdAtMs: number
  lastSeenAtMs?: number
}

/**
 * Whether a fill in agent mode may be a grant at all — off by default, and so
 * is agent access on every lockbox — and what "allow for N days" starts at.
 */
export interface AgentsSettings {
  enabled: boolean
  defaultStandingDays: number
}

/** The range a standing rule's length is held to, here and in Rust. */
export const MIN_STANDING_DAYS = 1
export const MAX_STANDING_DAYS = 90

export const browserStatus = (): Promise<BrowserStatus> => call('browser_status')

export const browserSetEnabled = (enabled: boolean): Promise<BrowserStatus> =>
  call('browser_set_enabled', { enabled })

export const browserSetKeepassxcCompat = (enabled: boolean): Promise<BrowserStatus> =>
  call('browser_set_keepassxc_compat', { enabled })

/** Turn agents on or off. Off ends every grant that is up. */
export const browserSetAgentsEnabled = (enabled: boolean): Promise<BrowserStatus> =>
  call('browser_set_agents_enabled', { enabled })

/** The grant dialog's default for "allow for N days"; Rust refuses outside 1–90. */
export const browserSetAgentsStandingDays = (days: number): Promise<BrowserStatus> =>
  call('browser_set_agents_standing_days', { days })

/**
 * How long Rust holds an `associate`, a passkey or a grant ask open for the
 * user (its `CONSENT_TIMEOUT`). Each dialog closes itself on the same clock,
 * so one left up is never answering an ask that has already been given up on.
 */
export const ASSOCIATE_TIMEOUT_MS = 60_000

/**
 * A client asking to connect: this ask's own `id`, its identification public
 * key, what it says it is, and — for an agent — the platform it runs on, for
 * the dialog to name it by.
 */
export interface AssociateAsk {
  id: string
  key: string
  kind: ClientKind
  platform?: string
}

/**
 * Answer the `browser:associate` ask `id`: the name to remember the extension
 * by, or null to refuse. Rust honours it only for the ask up under that id —
 * not for the same extension's next ask, which has an id of its own.
 */
export const browserRespond = (id: string, name: string | null): Promise<void> =>
  call('browser_respond', { id, name })

export const browserForgetClient = (key: string): Promise<BrowserStatus> =>
  call('browser_forget_client', { key })

/**
 * An account a sign-in could be as: one passkey the vault holds for the site,
 * and when it was added (RFC3339, when known), which is what tells two
 * passkeys a site named alike apart.
 */
export interface PasskeyAccount {
  userName: string
  userDisplayName: string
  createdAt?: string
}

/**
 * A page asking, through the extension, to create a passkey (`register`) or to
 * sign in with one (`get`). `rpId` is the site the passkey is for, `origin` the
 * page that asked; the account names come with a registration, as the site
 * gave them, and a sign-in lists the `accounts` the vault could sign in as,
 * newest first — the answer picks one by its place in the list.
 */
export interface PasskeyAsk {
  /** Names this ask; the answer carries it, and Rust honours it for this ask alone. */
  id: string
  kind: 'register' | 'get'
  rpId: string
  origin: string
  userName?: string
  userDisplayName?: string
  accounts?: PasskeyAccount[]
  /** The extension the ceremony came in through, by the name the user gave it. */
  requestedBy: { kind: ClientKind; name: string }
}

/** One of the page's logins an agent may be given: nothing secret. */
export interface GrantCandidate {
  id: string
  title: string
  username?: string | null
}

/**
 * A client asking to use a login on a page: the extension in agent mode, or
 * an agent that wants the extension to log it in. `clientName` is what the
 * user called it (or its key's two ends), `clientKind` what it is,
 * `candidates` the page's logins the policy admits — the dialog picks one.
 * An agent may say why (`reason`: its own words, shown as text and never
 * acted on); `endsSession` is the sign-out notice: when a fill grant ends, the
 * site's cookies go with it in the browser that filled, whichever client
 * asked. `stepUp` asks are
 * the ones a yes alone does not settle: `verify` says whether Rust will run
 * the biometric prompt or needs the master password sent with the answer.
 */
export interface AgentGrantAsk {
  /** Names this ask; the answer carries it, and Rust honours it for this ask alone. */
  id: string
  clientName: string
  clientKind: ClientKind
  origin: string
  mode: 'fill' | 'passkey' | 'header' | 'token' | 'env'
  reason?: string
  endsSession: boolean
  stepUp: boolean
  verify?: 'biometric' | 'password'
  candidates: GrantCandidate[]
  defaultStandingDays: number
}

/**
 * The yes to a grant ask: which candidate, and — for "allow for N days" — how
 * many, which writes a standing rule for this client, this login and this
 * site. `null` days is this once.
 */
export interface GrantAnswer {
  itemId: string
  standingDays: number | null
}

/**
 * The one decision the extension host has put to the user: an extension
 * asking to connect (`key` is its identification public key), a page's
 * passkey ceremony, or an agent's grant. Rust holds one at a time, in one
 * slot, whichever kind.
 */
export type BrowserConsent =
  | { kind: 'associate'; ask: AssociateAsk }
  | { kind: 'passkey'; ask: PasskeyAsk }
  | { kind: 'grant'; ask: AgentGrantAsk }

/**
 * Answer the `browser:passkey` ask `id`, with which of its accounts a sign-in
 * is as (the first when unsaid). Rust honours it only for the ask up under
 * that id.
 */
export const browserPasskeyRespond = (
  id: string,
  allow: boolean,
  account?: number
): Promise<void> => call('browser_passkey_respond', { id, allow, account })

/**
 * Answer the `agent:grant` ask `id`: the pick, or null to refuse. For a
 * step-up ask Rust verifies the person before the yes counts — the biometric
 * prompt, or `password` where the device has none — and a verification that
 * fails rejects here and refuses the ask. Rust honours the answer only for
 * the ask up under that id.
 */
export const agentGrantRespond = (
  id: string,
  answer: GrantAnswer | null,
  password?: string
): Promise<void> => call('agent_grant_respond', { id, answer, password })

/** A standing rule the user wrote from the grant dialog, as Settings lists it. */
export interface AgentRule {
  id: string
  clientKey: string
  clientName: string
  itemId: string | null
  itemTitle: string | null
  origin: string | null
  /** Unix ms; null is until removed. */
  expiresAtMs: number | null
  createdAtMs: number
}

/** A grant Rust still holds: active, or ended within the hour. */
export interface AgentGrant {
  id: string
  clientKey: string
  clientName: string
  itemId: string | null
  itemTitle: string | null
  origin: string | null
  state: 'pending' | 'active' | 'spent' | 'expired' | 'revoked' | 'denied'
  expiresAtMs: number
  uses: number
}

export const agentRules = (): Promise<AgentRule[]> => call('agent_rules')
export const agentGrants = (): Promise<AgentGrant[]> => call('agent_grants')
export const agentRevokeRule = (id: string): Promise<void> => call('agent_revoke_rule', { id })
export const agentEndGrant = (id: string): Promise<void> => call('agent_end_grant', { id })

/**
 * The command line tool (`rowel-cli`) as Rust finds it: `bundled` is the
 * binary beside the app when this build ships one (a release does; a dev
 * build only after `cargo build -p rowel-cli`), `installed` the `rowel` link
 * to it on the PATH when one is in place, `installDir` where a link goes or
 * is. Desktop only.
 */
export interface CliStatus {
  bundled: string | null
  installed: string | null
  installDir: string | null
}

export const cliStatus = (): Promise<CliStatus> => call('cli_status')

/**
 * Link the bundled tool into the PATH as `rowel`. Rust refuses with the
 * reason when nothing is bundled, when the name is taken by something that is
 * not a link of ours, or on Windows, where the folder goes on PATH by hand.
 */
export const cliInstall = (): Promise<CliStatus> => call('cli_install')
