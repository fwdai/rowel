import { describe, it, expect } from 'vitest'
import type { SyncStatus } from '@/api/sync'
import type { NearbyDevice, NearbyStatus } from '@/api/nearby'
import { initialApp } from '@/store'
import { nearbyUnavailable } from '@/test/ipc'
import { syncView } from './state'

const cloud = (s: Partial<SyncStatus> = {}): SyncStatus => ({ ...initialApp.sync, ...s })

const synced = (s: Partial<SyncStatus> = {}) =>
  cloud({ configured: true, lastSyncedAt: '2024-01-01T00:00:00.000Z', ...s })

const device = (state: NearbyDevice['state'], detail: string | null = null): NearbyDevice => ({
  id: state,
  name: 'Alex’s iPhone',
  platform: 'ios',
  state,
  lastSyncedAt: null,
  detail,
  detailKind: null
})

const nearby = (devices: NearbyDevice[], s: Partial<NearbyStatus> = {}): NearbyStatus => ({
  ...nearbyUnavailable(),
  available: true,
  enabled: true,
  devices,
  seq: 1,
  ...s
})

describe('syncView', () => {
  describe('with a cloud account', () => {
    // Every cloud state, with a nearby status beside it that would say
    // something else: the account speaks for the vault either way.
    const busyNearby = nearby([device('syncing'), device('error', 'Clocks differ')])

    it.each([
      ['a run in flight', synced({ inProgress: true }), { tone: 'loading', message: 'Syncing…' }],
      [
        'a failed run',
        synced({ error: 'Drive said no' }),
        { tone: 'bad', message: 'Something went wrong', detail: 'Drive said no' }
      ],
      ['no run yet', cloud({ configured: true }), { tone: 'idle', message: 'Waiting to sync' }],
      ['a landed run', synced(), { tone: 'good', message: 'Sync Successful' }]
    ])('reads %s exactly as before', (_, sync, expected) => {
      expect(syncView(sync, null)).toEqual(expected)
      expect(syncView(sync, busyNearby)).toEqual(expected)
    })
  })

  describe('without one', () => {
    const local = { tone: 'local', message: 'Changes are saved on this device only' }

    it('is this device only before nearby has reported, without it, or with it off', () => {
      expect(syncView(cloud(), null)).toEqual(local)
      expect(syncView(cloud(), nearby([device('upToDate')], { available: false }))).toEqual(local)
      expect(syncView(cloud(), nearby([device('upToDate')], { enabled: false }))).toEqual(local)
    })

    it('waits for a nearby device while none is up to date', () => {
      const waiting = { tone: 'idle', message: 'Waiting for a nearby device' }
      expect(syncView(cloud(), nearby([]))).toEqual(waiting)
      expect(
        syncView(cloud(), nearby([device('offline'), device('paused'), device('waiting')]))
      ).toEqual(waiting)
    })

    it('is up to date once a nearby device is', () => {
      expect(syncView(cloud(), nearby([device('offline'), device('upToDate')]))).toEqual({
        tone: 'good',
        message: 'Up to date with nearby devices'
      })
    })

    it('spins while a nearby device syncs, even beside one in trouble', () => {
      const view = { tone: 'loading', message: 'Syncing with a nearby device…' }
      expect(syncView(cloud(), nearby([device('upToDate'), device('syncing')]))).toEqual(view)
      expect(syncView(cloud(), nearby([device('error', 'x'), device('syncing')]))).toEqual(view)
    })

    it('says a nearby device needs the new master password', () => {
      expect(syncView(cloud(), nearby([device('upToDate'), device('needsPassword')]))).toEqual({
        tone: 'bad',
        message: 'A nearby device needs the new master password'
      })
    })

    it('asks for the master password a nearby device changed, before any other trouble', () => {
      const devices = [device('needsPassword'), device('passwordChanged')]
      expect(syncView(cloud(), nearby(devices))).toEqual({
        tone: 'bad',
        message: 'Master password changed on another device',
        adopt: true
      })
    })

    it('says what went wrong with a nearby device in its own words', () => {
      expect(
        syncView(cloud(), nearby([device('clockSkew', 'The clocks are 7 minutes apart')]))
      ).toEqual({
        tone: 'bad',
        message: 'Something went wrong',
        detail: 'The clocks are 7 minutes apart'
      })
      expect(syncView(cloud(), nearby([device('error', 'The connection dropped')]))).toEqual({
        tone: 'bad',
        message: 'Something went wrong',
        detail: 'The connection dropped'
      })
    })
  })
})
