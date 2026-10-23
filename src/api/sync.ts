import { call } from './client'
import { describeError, type BackendErrorKind } from './errors'
import type { Provider } from './provider'

/**
 * The whole of what the frontend knows about sync, carried by every
 * `sync:status` event (see `events.ts`). The store keeps it verbatim: the
 * backend owns every flow — it opens the browser, hears back from it, runs the
 * sync — so it is the one that can say.
 */
export interface SyncStatus {
  /** This vault has a provider connected. */
  configured: boolean
  /** Which one; null exactly when `configured` is false. */
  provider: Provider | null
  /** A consent flow is out with the browser. */
  pending: boolean
  inProgress: boolean
  /** What the last connect or run failed with, until the next one starts. */
  error: string | null
  /**
   * The kind of that failure, always set alongside `error`. A failed run is
   * reported as status rather than as a rejected promise, so this is what lets
   * a screen recognise one particular failure without reading the English
   * sentence Rust built — the same contract every rejection has.
   */
  errorKind: BackendErrorKind | null
  /** ISO time of the last run that succeeded in this process, or null. */
  lastSyncedAt: string | null
  /**
   * The backend's count of transitions so far, monotonic within the process.
   * Two snapshots reach the store by different routes — the `sync:status`
   * event and the `app_status` probe — and can arrive out of order; the store
   * keeps whichever has the higher `seq` (`setSyncStatus`).
   */
  seq: number
}

/**
 * The last failure as copy the user can read, or null when there was none.
 * `describeError` over the two halves the status carries, so a sync failure
 * reads the same as the identical failure raised from a command.
 */
export const syncErrorText = (sync: SyncStatus): string | null =>
  sync.error === null
    ? null
    : describeError({ kind: sync.errorKind ?? 'other', message: sync.error })

/**
 * The account holds a deletion marker for this vault — another device deleted
 * it everywhere, or someone with access to the account planted one — so its
 * runs stop here until the user either removes it from this device too
 * (`workspaceDelete`) or keeps it and puts it back (`syncReupload`).
 */
export const syncDeletedRemotely = (sync: SyncStatus): boolean =>
  sync.errorKind === 'vaultDeletedRemotely'

/**
 * The account's pack is sealed under a newer master password than this
 * device's — another device changed it — so runs stop here until the user
 * types the new one in (`syncAdoptPassword`).
 */
export const syncPasswordChangedRemotely = (sync: SyncStatus): boolean =>
  sync.errorKind === 'passwordChangedRemotely'

/**
 * The account holds this vault under a password this device has already
 * replaced and no longer has the key for, so runs stop here without pushing.
 * Either the device that wrote it adopts the new password on its next run, or
 * the user overwrites the account's copy with this one (`syncReplaceRemote`).
 */
export const syncUnderRetiredPassword = (sync: SyncStatus): boolean =>
  sync.errorKind === 'vaultUnderRetiredPassword'

/**
 * The provider this vault syncs through, or null when it does not. A backend
 * from before there was a choice reports a connection with no provider, and
 * the only one it ever spoke to was Google Drive.
 */
export const syncProvider = (sync: SyncStatus): Provider | null =>
  sync.configured ? (sync.provider ?? 'googleDrive') : null

/**
 * Start `provider`'s consent flow for the open vault. Returns immediately on
 * every platform: the backend probes the account first and reports on
 * `setup:account:pending`, then one of `setup:account:probed` /
 * `setup:account:error` — the same events the first run and Settings ›
 * Workspaces listen to. A rejection here is an immediate guard failure
 * (another setup step running) and has already been reported as `sync:status`.
 *
 * A probe leaves the account's tokens pending and its vaults listed; whether
 * this vault may join the account is then `syncAdoptPending`'s to say.
 */
export const syncConnect = (provider: Provider): Promise<void> =>
  call('sync_connect', { provider })

export const syncDisconnect = (): Promise<void> => call('sync_disconnect')

export const syncNow = (): Promise<void> => call('sync_now')

/**
 * Keep a vault the account says was deleted: the backend takes the deletion
 * marker out of the account and starts a run, which syncs with the pack if it
 * is still there and uploads this copy if it is not. Reports through
 * `sync:status` like any run.
 */
export const syncReupload = (): Promise<void> => call('sync_reupload')

/**
 * Switch the open vault to the master password another device changed it to.
 * The backend checks `password` by opening the account's pack with it, re-keys
 * this vault (and every lockbox beside it on the same old password) and starts
 * a run. Resolves to the ids of lockboxes beside it that it could not move,
 * like `changeMasterPassword`. A wrong password rejects with `invalidPassword`
 * and counts against the unlock backoff (`tooManyAttempts`).
 */
export const syncAdoptPassword = (password: string): Promise<string[]> =>
  call('sync_adopt_password', { password })

/**
 * Overwrite the account's copy of the open vault, held under a password this
 * device has retired, with this device's, then start a run. Edits only the
 * account's copy had are lost; the device that wrote them is asked for the new
 * password on its next run.
 */
export const syncReplaceRemote = (): Promise<void> => call('sync_replace_remote')

/**
 * Ask the backend to make the account a probe left pending this vault's. It
 * lists the account's vaults with the pending tokens and decides: an account
 * holding no vault takes this one as its first, and one already holding this
 * vault's id takes it as another device of it — the tokens are sealed under the
 * open vault's key and the first sync runs, reporting through `sync:status`
 * like a connect. An account holding only *other* vaults rejects with
 * `vaultNotInAccount` and leaves the tokens pending, so the caller offers those
 * vaults to restore (`workspaceRestoreFromPending`) instead of adding a pack
 * beside them.
 */
export const syncAdoptPending = (): Promise<void> => call('sync_adopt_pending')
