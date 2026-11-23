import type { TKey } from '@/i18n'
import { syncErrorText, type SyncStatus } from '@/api/sync'
import type { NearbyDevice, NearbyStatus } from '@/api/nearby'

export type SyncTone = 'local' | 'idle' | 'loading' | 'good' | 'bad'

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
  /** The chip opens the prompt for the master password another device changed. */
  adopt?: true
}

/**
 * The chip's five states, in priority order, from the backend's sync status
 * and, where no cloud account is connected, nearby sync's.
 *
 * Ordering matters: a run in flight outranks the previous run's verdict, so a
 * retry after a failure reads as "syncing" rather than staying red until it
 * lands. `lastSyncedAt` is what promotes the chip to "good": a vault connected
 * a second ago has not synced yet, and must not claim to be up to date.
 *
 * A connected cloud account speaks for the vault whatever nearby sync is
 * doing; nearby only replaces "this device only", which it makes untrue.
 */
export const syncView = (sync: SyncStatus, nearby: NearbyStatus | null): SyncView => {
  if (!sync.configured)
    return (
      nearbyView(nearby) ?? { tone: 'local', message: 'Changes are saved on this device only' }
    )
  if (sync.inProgress) return { tone: 'loading', message: 'Syncing…' }
  if (sync.error !== null)
    return {
      tone: 'bad',
      message: 'Something went wrong',
      detail: syncErrorText(sync) ?? undefined
    }
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
  adopt: true
} as const

/**
 * Nearby sync as the chip says it, or null when it is off (or this build has
 * none). The same order as the cloud's: a run in flight, then a device that
 * cannot sync until something is done about it, then whether any device is
 * here and current. A paused device is the user's choice and counts as quiet.
 */
const nearbyView = (nearby: NearbyStatus | null): SyncView | null => {
  if (!nearby?.available || !nearby.enabled) return null
  const { devices } = nearby
  if (devices.some(device => device.state === 'syncing'))
    return { tone: 'loading', message: 'Syncing with a nearby device…' }
  if (devices.some(device => device.state === 'passwordChanged')) return PASSWORD_CHANGED
  const trouble = devices.find(device => TROUBLE.includes(device.state))
  if (trouble?.state === 'needsPassword')
    return { tone: 'bad', message: 'A nearby device needs the new master password' }
  if (trouble)
    return { tone: 'bad', message: 'Something went wrong', detail: trouble.detail ?? undefined }
  if (devices.some(device => device.state === 'upToDate'))
    return { tone: 'good', message: 'Up to date with nearby devices' }
  return { tone: 'idle', message: 'Waiting for a nearby device' }
}
