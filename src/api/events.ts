import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type { PasskeyAsk } from './browser'
import type { SetupDriveFile } from './setup'
import type { SyncStatus } from './sync'
import type { EntryMeta } from './types'

/**
 * The event catalog. The backend `emit`s these; the frontend `listen`s. Used for
 * background work (sync, auto-lock, a long import) that is not a direct
 * request/response command.
 *
 * `setup:drive:pending` → `setup:drive:probed` | `setup:drive:error` is a
 * consent flow out with the browser against an account there is no vault behind
 * yet. The backend owns all three: it is what opens the browser and what hears
 * back from it, so the frontend mirrors these rather than guessing from a click
 * — the command itself resolves the moment the browser is on screen. An empty
 * `files` there is an account with no Rowel data in it, a fact rather than a
 * failure, so it is not an error event.
 */
export interface EventPayloads {
  'vault:locked': void
  /**
   * A sync pulled entries this device did not have. Carries the whole refreshed
   * list rather than a "reload" ping so the store updates in one render, and
   * because the backend has already paid for the query.
   */
  'vault:merged': { entries: EntryMeta[] }
  /**
   * The whole of sync, every time anything about it changes: a consent flow
   * opening or closing, a run starting or ending. The frontend stores it as-is
   * — there is no sequence of events to reconstruct state from.
   */
  'sync:status': SyncStatus
  'import:progress': { done: number; total: number }
  'setup:drive:pending': void
  /** Every vault the account holds, newest first; empty is "nothing to restore". */
  'setup:drive:probed': { files: SetupDriveFile[] }
  'setup:drive:error': { error: string }
  /**
   * After every sync run: the account's vaults that no workspace on this
   * device holds, newest first, in the probe's shape because Settings ›
   * Workspaces offers them through the same picker. Empty means every vault in
   * the account is already here.
   */
  'workspaces:remote': { files: SetupDriveFile[] }
  /**
   * A vault from the account was added as a workspace without being asked
   * for: the password that just opened a vault opened it too. Nothing has
   * switched; the list of workspaces has grown by `name`.
   */
  'workspaces:added': { name: string }
  /**
   * A sync pulled a newer name for the open workspace — it was renamed on
   * another device. No payload: names live on `app_status`, so the frontend
   * re-reads that and the header and Workspaces list redraw from it.
   */
  'workspaces:renamed': void
  /**
   * The OS opened a backup with the app — a double-clicked `.rowel` or
   * `.swftx`. Rust also parks the path for a shell that was not yet listening
   * (`takeOpenedFile` in `api/app`), so a launch by double-click and an open
   * while running both end up in `store/app`'s `fileOpened`.
   */
  'file:opened': { path: string }
  /**
   * A browser extension asks to be let in. `key` is its identification public
   * key, base64, for the consent dialog to show a fingerprint of; `id` names
   * this ask, and the answer goes back through `browser_respond` under it, so
   * a dialog left up past its ask cannot answer the next. Desktop only.
   */
  'browser:associate': { id: string; key: string }
  /**
   * A page asks, through the extension, for a passkey ceremony. Rust holds the
   * request open until `browser_passkey_respond`, or a minute. Desktop only.
   */
  'browser:passkey': PasskeyAsk
  /**
   * An extension was let into the open vault by the consent dialog. Settings ›
   * Browser extension re-reads its status on it; no payload, since that status
   * is the one answer its list is drawn from. Desktop only.
   */
  'browser:clients': void
}

export type EventName = keyof EventPayloads

type Camel<S extends string> = S extends `${infer Head}:${infer Tail}`
  ? `${Head}${Capitalize<Camel<Tail>>}`
  : S

/**
 * Call-site names for the events. Derived from `EventPayloads`, so adding an
 * event without a key here — or keeping a key whose event is gone — fails the
 * build rather than drifting.
 */
export const EVENTS: { [K in EventName as Camel<K>]: K } = {
  vaultLocked: 'vault:locked',
  vaultMerged: 'vault:merged',
  syncStatus: 'sync:status',
  importProgress: 'import:progress',
  setupDrivePending: 'setup:drive:pending',
  setupDriveProbed: 'setup:drive:probed',
  setupDriveError: 'setup:drive:error',
  workspacesRemote: 'workspaces:remote',
  workspacesAdded: 'workspaces:added',
  workspacesRenamed: 'workspaces:renamed',
  fileOpened: 'file:opened',
  browserAssociate: 'browser:associate',
  browserPasskey: 'browser:passkey',
  browserClients: 'browser:clients'
}

// Typed wrapper over Tauri's `listen`.
export const on = <E extends EventName>(
  event: E,
  handler: (payload: EventPayloads[E]) => void
): Promise<UnlistenFn> => listen<EventPayloads[E]>(event, e => handler(e.payload))
