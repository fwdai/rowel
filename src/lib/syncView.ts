import type { TKey } from '@/i18n'
import {
  syncDeletedRemotely,
  syncErrorText,
  syncPasswordChangedRemotely,
  syncUnderRetiredPassword,
  type SyncStatus
} from '@/api/sync'
import type { NearbyDevice, NearbyStatus } from '@/api/nearby'

export type SyncTone = 'local' | 'idle' | 'loading' | 'good' | 'bad'

/**
 * Where a failure is put right, which is not always where it is shown:
 * `password` is the prompt for the master password another device changed;
 * `lockboxes` is Settings › Lockboxes, whose row holds the choices for a vault
 * deleted from the account or left there under a retired password; `cloud`
 * and `nearby` are the pages of Settings › Sync & backup that own the
 * connection and the device.
 */
export type SyncFix = 'password' | 'lockboxes' | 'cloud' | 'nearby'

export interface SyncView {
  tone: SyncTone
  /**
   * A catalogue key, not a sentence. Typed as `TKey`, so a message with no
   * en-US entry fails the build rather than reaching a user in English. That
   * type is the whole guard here: the i18n suite scans for literal translation
   * calls, so a sentence picked in this module and translated from a variable
   * at the call site would be invisible to it.
   */
  message: TKey
  /**
   * What the failure was, shown in place of `message`. Translated where the
   * backend named a kind the catalogues cover (`syncErrorText`) and the
   * backend's own words otherwise. Either way it is a sentence, not a
   * catalogue key, which is what lets `message` stay honestly typed instead of
   * casting an arbitrary string to `TKey`.
   */
  detail?: string
  /** Set with every `bad`: the one place that says where each failure goes. */
  fix?: SyncFix
}

/**
 * Where the vault stands: the header chip's state, which the overview's status
 * card spells out, in the words of whichever way of syncing speaks for it.
 *
 * A connected cloud account speaks for the vault whatever nearby sync is
 * doing. A connect still under way, or one that failed, does not: there is no
 * account yet. Nearby then only replaces "this device only", which it makes
 * untrue.
 */
export const syncView = (sync: SyncStatus, nearby: NearbyStatus | null): SyncView =>
  (sync.configured ? cloudView(sync) : nearbyView(nearby)) ?? {
    tone: 'local',
    message: 'Changes are saved on this device only'
  }

/**
 * Where the cloud account stands, or null with no account and none under way.
 * The one reading of it: the chip and the status card take it through
 * `syncView`, and the overview's row and the account card's pill and figures
 * put it in their own words.
 *
 * Ordering matters: a run in flight outranks the previous run's verdict, so a
 * retry after a failure reads as "syncing" rather than staying red until it
 * lands. Both outrank whether an account is connected yet, since a connect
 * runs and fails as a sync does. `lastSyncedAt` is what promotes the account
 * to "good": a vault connected a second ago has not synced yet, and must not
 * claim to be up to date.
 */
export const cloudView = (sync: SyncStatus): SyncView | null => {
  if (sync.inProgress) return { tone: 'loading', message: 'Syncing…' }
  if (sync.error !== null)
    return {
      tone: 'bad',
      message: 'Something went wrong',
      detail: syncErrorText(sync) ?? undefined,
      fix: syncPasswordChangedRemotely(sync)
        ? 'password'
        : syncDeletedRemotely(sync) || syncUnderRetiredPassword(sync)
          ? 'lockboxes'
          : 'cloud'
    }
  if (!sync.configured) return null
  // Connected, but nothing has landed yet. Deliberately not `loading`: if the
  // first run never starts, a spinner here would turn on at unlock and never
  // stop. A quiet cloud with no badge claims nothing instead.
  if (!sync.lastSyncedAt) return { tone: 'idle', message: 'Waiting to sync' }
  return { tone: 'good', message: 'Sync Successful' }
}

const TROUBLE: NearbyDevice['state'][] = ['needsPassword', 'clockSkew', 'error']
const PASSWORD_CHANGED = {
  tone: 'bad',
  message: 'Master password changed on another device',
  fix: 'password'
} as const

/**
 * Where nearby sync stands, or null when it is off (or this build has none).
 * Read by the chip and the status card where no cloud account speaks for the
 * vault, and by the overview's Nearby devices row always.
 *
 * Its status is per device, across every lockbox this one holds, so it claims
 * nothing for the open lockbox: it says that nearby sync is on, and what needs
 * doing about a device. Whether a device is current is said in Settings, on
 * its row.
 */
export const nearbyView = (nearby: NearbyStatus | null): SyncView | null => {
  if (!nearby?.available || !nearby.enabled) return null
  const { devices } = nearby
  if (devices.some(device => device.state === 'passwordChanged')) return PASSWORD_CHANGED
  const trouble = devices.find(device => TROUBLE.includes(device.state))
  if (trouble?.state === 'needsPassword')
    return {
      tone: 'bad',
      message: 'A nearby device needs the new master password',
      fix: 'nearby'
    }
  if (trouble)
    return {
      tone: 'bad',
      message: 'Something went wrong',
      detail: trouble.detail ?? undefined,
      fix: 'nearby'
    }
  return { tone: 'idle', message: 'Nearby sync is on' }
}
