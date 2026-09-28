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
 * How long Rust holds an `associate` ask open for the user (its
 * `CONSENT_TIMEOUT`). The dialog closes itself on the same clock, so one left
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
