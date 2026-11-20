import { call } from './client'
import { describeError, type BackendErrorKind } from './errors'

/**
 * Nearby sync: two Rowel apps on the same network, both open and unlocked,
 * syncing their lockboxes directly, beside cloud sync rather than instead of
 * it. The backend owns discovery, trust and every exchange; the frontend sees
 * one status, carried whole by every `nearby:status` event (see `events.ts`)
 * and kept by the store as-is.
 */
export interface NearbyStatus {
  /** This build can do it at all. */
  available: boolean
  enabled: boolean
  /** What this device is called on the network. */
  deviceName: string
  devices: NearbyDevice[]
  /**
   * The backend's count of transitions, monotonic within the process. The
   * event and the `nearby_status` read can arrive out of order; the store
   * keeps whichever has the higher `seq` (`setNearbyStatus`).
   */
  seq: number
}

export type NearbyPlatform = 'macos' | 'windows' | 'linux' | 'ios'

/**
 * `syncing`       — a run is in flight
 * `upToDate`      — connected, nothing to send
 * `offline`       — known, not on the network now
 * `paused`        — this device stopped syncing with it (`nearbyPause`)
 * `needsPassword` — the master password changed on one side
 * `clockSkew`     — the two clocks are too far apart; `detail` says by how much
 * `error`         — anything else; `detail` says what, `detailKind` which kind
 */
export type NearbyDeviceState =
  | 'syncing'
  | 'upToDate'
  | 'offline'
  | 'paused'
  | 'needsPassword'
  | 'clockSkew'
  | 'error'

/** One of the owner's other devices: one that proved it holds a lockbox key. */
export interface NearbyDevice {
  id: string
  name: string
  platform: NearbyPlatform
  state: NearbyDeviceState
  /** ISO time of the last exchange that converged, or null. */
  lastSyncedAt: string | null
  /** The backend's sentence, for `clockSkew` and `error`. */
  detail: string | null
  /**
   * Its kind, as `SyncStatus.errorKind` is to `error`: what gets translated
   * (`describeError`), with `detail` the fallback when there is none.
   */
  detailKind: BackendErrorKind | null
}

/**
 * What a device's last failure was, as copy the user can read, or null when
 * the backend said nothing. `describeError` over the two halves, as
 * `syncErrorText` is for the account: the kind in the user's language, and
 * the backend's sentence where the kind has nothing to translate.
 */
export const nearbyDetailText = (device: NearbyDevice): string | null =>
  device.detail === null && device.detailKind === null
    ? null
    : describeError({ kind: device.detailKind ?? 'other', message: device.detail ?? '' }) ||
      device.detail

export const nearbyStatus = (): Promise<NearbyStatus> => call('nearby_status')

export const nearbySetEnabled = (enabled: boolean): Promise<void> =>
  call('nearby_set_enabled', { enabled })

/** Stop syncing with one device, or start again. Local to this device. */
export const nearbyPause = (deviceId: string, paused: boolean): Promise<void> =>
  call('nearby_pause', { deviceId, paused })

/**
 * Refuse one device from now on, here and — through the lockbox — on the
 * owner's other devices. What that device already holds stays on it.
 */
export const nearbyRemove = (deviceId: string): Promise<void> =>
  call('nearby_remove', { deviceId })
