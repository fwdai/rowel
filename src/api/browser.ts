import { call } from './client'

/**
 * The browser extension host (desktop only): Rowel speaks the KeePassXC-Browser
 * protocol, so the stock extension fills logins from the vault. Every command
 * that changes something answers with the whole status as it now stands, so
 * Settings redraws from what Rust says rather than from a guess.
 */
export interface BrowserStatus {
  enabled: boolean
  /**
   * Every browser Rust knows how to register with. `detected` is its profile
   * directory being there; `installed` is the manifest it finds Rowel by, which
   * is written on enabling and only for a detected browser; `conflict` is that
   * place already held by KeePassXC itself, which Rowel leaves alone.
   */
  browsers: {
    id: string
    label: string
    detected: boolean
    installed: boolean
    conflict: boolean
  }[]
  /**
   * The extensions let into the open vault: the name they were given, and
   * their public key. Each vault keeps its own list.
   */
  clients: { name: string; key: string }[]
}

export type BrowserClient = BrowserStatus['clients'][number]

export const browserStatus = (): Promise<BrowserStatus> => call('browser_status')

export const browserSetEnabled = (enabled: boolean): Promise<BrowserStatus> =>
  call('browser_set_enabled', { enabled })

/**
 * How long Rust holds an `associate` or a passkey ask open for the user (its
 * `CONSENT_TIMEOUT`). Each dialog closes itself on the same clock, so one left
 * up is never answering an ask that has already been given up on.
 */
export const ASSOCIATE_TIMEOUT_MS = 60_000

/** An extension asking to connect: this ask's own `id`, and its identification public key. */
export interface AssociateAsk {
  id: string
  key: string
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
}

/**
 * The one decision the extension host has put to the user: an extension
 * asking to connect (`key` is its identification public key), or a page's
 * passkey ceremony. Rust holds one at a time, in one slot, whichever kind.
 */
export type BrowserConsent =
  | { kind: 'associate'; ask: AssociateAsk }
  | { kind: 'passkey'; ask: PasskeyAsk }

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
