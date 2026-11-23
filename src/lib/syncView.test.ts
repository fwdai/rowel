import { describe, it, expect } from 'vitest'
import type { SyncStatus } from '@/api/sync'
import type { NearbyDevice, NearbyStatus } from '@/api/nearby'
import { initialApp } from '@/store'
import { nearbyUnavailable } from '@/test/ipc'
import { cloudView, nearbyView, syncView } from './syncView'

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
        { tone: 'bad', message: 'Something went wrong', detail: 'Drive said no', fix: 'cloud' }
      ],
      ['no run yet', cloud({ configured: true }), { tone: 'idle', message: 'Waiting to sync' }],
      ['a landed run', synced(), { tone: 'good', message: 'Sync Successful' }]
    ])('reads %s exactly as before', (_, sync, expected) => {
      expect(syncView(sync, null)).toEqual(expected)
      expect(syncView(sync, busyNearby)).toEqual(expected)
    })
  })

  // A failure is put right where its controls are, which the view names once
  // for the chip and for Settings › Sync & backup alike.
  describe('where a cloud failure is put right', () => {
    it.each([
      ['passwordChangedRemotely', 'password'],
      ['vaultDeletedRemotely', 'lockboxes'],
      ['vaultUnderRetiredPassword', 'lockboxes'],
      ['other', 'cloud'],
      [null, 'cloud']
    ] as const)('sends %s to %s', (errorKind, fix) => {
      expect(syncView(synced({ error: 'stopped', errorKind }), null).fix).toBe(fix)
    })

    it('names nowhere while nothing is wrong', () => {
      expect(syncView(synced(), null).fix).toBeUndefined()
      expect(syncView(cloud(), null).fix).toBeUndefined()
    })
  })

  describe('without one', () => {
    const local = { tone: 'local', message: 'Changes are saved on this device only' }

    it('is this device only before nearby has reported, without it, or with it off', () => {
      expect(syncView(cloud(), null)).toEqual(local)
      expect(syncView(cloud(), nearby([device('upToDate')], { available: false }))).toEqual(local)
      expect(syncView(cloud(), nearby([device('upToDate')], { enabled: false }))).toEqual(local)
    })

    // The status is per device, not per lockbox: the chip claims nothing for
    // the open one, whatever a device shared through another lockbox is doing.
    it('says only that nearby sync is on, whatever its devices are doing', () => {
      const on = { tone: 'idle', message: 'Nearby sync is on' }
      expect(syncView(cloud(), nearby([]))).toEqual(on)
      expect(
        syncView(cloud(), nearby([device('offline'), device('paused'), device('waiting')]))
      ).toEqual(on)
      expect(syncView(cloud(), nearby([device('upToDate'), device('syncing')]))).toEqual(on)
    })

    it('says a nearby device needs the new master password', () => {
      expect(syncView(cloud(), nearby([device('upToDate'), device('needsPassword')]))).toEqual({
        tone: 'bad',
        message: 'A nearby device needs the new master password',
        fix: 'nearby'
      })
    })

    it('asks for the master password a nearby device changed, before any other trouble', () => {
      const devices = [device('needsPassword'), device('passwordChanged')]
      expect(syncView(cloud(), nearby(devices))).toEqual({
        tone: 'bad',
        message: 'Master password changed on another device',
        fix: 'password'
      })
    })

    it('says what went wrong with a nearby device in its own words', () => {
      expect(
        syncView(cloud(), nearby([device('clockSkew', 'The clocks are 7 minutes apart')]))
      ).toEqual({
        tone: 'bad',
        message: 'Something went wrong',
        detail: 'The clocks are 7 minutes apart',
        fix: 'nearby'
      })
      expect(syncView(cloud(), nearby([device('error', 'The connection dropped')]))).toEqual({
        tone: 'bad',
        message: 'Something went wrong',
        detail: 'The connection dropped',
        fix: 'nearby'
      })
    })
  })
})

// Each method's own standing, which the overview's rows and the account card
// read whether or not that method is the one speaking for the vault.
describe('cloudView', () => {
  it('is nothing with no account and none under way', () => {
    expect(cloudView(cloud())).toBeNull()
  })

  it('is the vault’s view once an account is connected', () => {
    for (const sync of [synced(), synced({ inProgress: true }), synced({ error: 'no' })])
      expect(cloudView(sync)).toEqual(syncView(sync, null))
  })

  // A connect runs and fails like a sync, so the account's own surfaces say
  // so; the vault's view does not, since there is no account yet.
  it('carries a connect under way, or one that failed, which the vault’s view leaves out', () => {
    const connecting = cloud({ inProgress: true })
    const refused = cloud({ error: 'access_denied' })

    expect(cloudView(connecting)?.tone).toBe('loading')
    expect(cloudView(refused)).toMatchObject({ tone: 'bad', fix: 'cloud' })
    expect(syncView(connecting, null).tone).toBe('local')
    expect(syncView(refused, null).tone).toBe('local')
  })
})

describe('nearbyView', () => {
  it('is nothing while nearby sync is off or absent', () => {
    expect(nearbyView(null)).toBeNull()
    expect(nearbyView(nearby([], { enabled: false }))).toBeNull()
    expect(nearbyView(nearby([], { available: false }))).toBeNull()
  })

  // The vault's view leaves nearby out under a cloud account; nearby's own
  // does not, so a device in trouble is still said where nearby is.
  it('says a device is in trouble whatever account speaks for the vault', () => {
    const status = nearby([device('upToDate'), device('needsPassword')])

    expect(syncView(synced(), status).tone).toBe('good')
    expect(nearbyView(status)).toEqual({
      tone: 'bad',
      message: 'A nearby device needs the new master password',
      fix: 'nearby'
    })
  })
})
