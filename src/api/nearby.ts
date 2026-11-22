import { call } from './client'
import { describeError, type BackendErrorKind } from './errors'

export interface NearbyStatus {
  available: boolean
  enabled: boolean
  deviceName: string
  devices: NearbyDevice[]
  seq: number
}

export type NearbyPlatform = 'macos' | 'windows' | 'linux' | 'ios'

export type NearbyDeviceState =
  | 'paused'
  | 'offline'
  | 'syncing'
  | 'waiting'
  | 'upToDate'
  | 'needsPassword'
  | 'clockSkew'
  | 'error'

export interface NearbyDevice {
  id: string
  name: string
  platform: NearbyPlatform
  state: NearbyDeviceState
  lastSyncedAt: string | null
  detail: string | null
  detailKind: BackendErrorKind | null
}

export const nearbyDetailText = (device: NearbyDevice): string | null =>
  device.detail === null && device.detailKind === null
    ? null
    : describeError({ kind: device.detailKind ?? 'other', message: device.detail ?? '' }) ||
      device.detail

export const nearbyStatus = (): Promise<NearbyStatus> => call('nearby_status')

export const nearbySetEnabled = (enabled: boolean): Promise<void> =>
  call('nearby_set_enabled', { enabled })

export const nearbyPause = (deviceId: string, paused: boolean): Promise<void> =>
  call('nearby_pause', { deviceId, paused })

export const nearbyRemove = (deviceId: string): Promise<void> =>
  call('nearby_remove', { deviceId })
