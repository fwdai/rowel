import { describe, it, expect } from 'vitest'
import { act, fireEvent, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Settings from '@/components/Main/Sidebar/Settings'
import type { NearbyDevice, NearbyStatus } from '@/api/nearby'
import { openSettings, setNearbyStatus, useUi } from '@/store'
import { calls, mockCommand, mockCommandOnce, nearbyUnavailable } from './ipc'

/*
 * Settings › Sync & backup › Nearby devices, against the IPC mock. Every
 * change the page asks for answers through `nearby:status`, so each spec
 * reports the status the backend would have sent next (`report`).
 */

const DAY = 24 * 60 * 60 * 1000

let seq = 0

const report = (status: Partial<NearbyStatus> = {}) =>
  act(() =>
    setNearbyStatus({
      ...nearbyUnavailable(),
      available: true,
      enabled: true,
      deviceName: 'Studio Mac',
      ...status,
      seq: ++seq
    })
  )

const device = (over: Partial<NearbyDevice> = {}): NearbyDevice => ({
  id: 'd1',
  name: 'Alex’s iPhone',
  platform: 'ios',
  state: 'upToDate',
  lastSyncedAt: null,
  detail: null,
  detailKind: null,
  ...over
})

const openSync = () => {
  openSettings('sync')
  render(<Settings />)
}

// The overview's row is the way in, and is only there on a build with nearby.
const row = () => screen.queryByTestId('settings-sync-open-nearby')
const openNearby = () => {
  openSync()
  fireEvent.click(screen.getByTestId('settings-sync-open-nearby'))
}

const toggle = () => screen.getByTestId('settings-nearby-toggle')
const status = (id = 'd1') => screen.getByTestId(`settings-nearby-status-${id}`)

describe('Settings › Sync › Nearby devices', () => {
  it('draws nothing before the backend has said anything', () => {
    openSync()
    expect(screen.getByTestId('settings-sync-open-cloud')).toBeInTheDocument()
    expect(screen.queryByText('Nearby devices')).not.toBeInTheDocument()
    expect(row()).not.toBeInTheDocument()
  })

  it('draws nothing on a build without it', () => {
    act(() => setNearbyStatus({ ...nearbyUnavailable(), enabled: true, seq: 1 }))
    openSync()
    expect(row()).not.toBeInTheDocument()
  })

  it('sits above the cloud account on the overview, saying whether it is on', () => {
    report({ enabled: false })
    openSync()

    const cloud = screen.getByTestId('settings-sync-open-cloud')
    expect(row()!.compareDocumentPosition(cloud) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(screen.getByTestId('settings-sync-open-nearby-status')).toHaveTextContent('Off')

    report({ enabled: true })
    expect(screen.getByTestId('settings-sync-open-nearby-status')).toHaveTextContent('On')
    // With no cloud account, nearby sync is what the summary speaks for.
    expect(screen.getByTestId('settings-sync-summary')).toHaveTextContent('Nearby sync is on')
  })

  it('leaves the cloud account’s page as it was', () => {
    report({ enabled: false })
    openSync()
    fireEvent.click(screen.getByTestId('settings-sync-open-cloud'))

    expect(screen.getByTestId('settings-account-name')).toHaveTextContent('Cloud sync')
    expect(screen.getByTestId('settings-drive-connect')).toBeInTheDocument()
    expect(screen.queryByTestId('settings-nearby-toggle')).not.toBeInTheDocument()
  })

  it('steps back to the overview when the build stops offering it', () => {
    report({ enabled: true })
    openNearby()
    expect(toggle()).toBeInTheDocument()

    act(() => setNearbyStatus({ ...nearbyUnavailable(), seq: ++seq }))
    expect(screen.queryByTestId('settings-nearby-toggle')).not.toBeInTheDocument()
    expect(screen.getByTestId('settings-sync-open-cloud')).toBeInTheDocument()
  })

  it('names this device, the way the others see it', () => {
    report({ devices: [device()] })
    openNearby()
    expect(screen.getByTestId('settings-nearby-this-device')).toHaveTextContent('Studio Mac')
  })

  it('says what it does under the switch, and shows nothing else while off', () => {
    report({ enabled: false, devices: [device()] })
    openNearby()

    expect(toggle()).toHaveAttribute('aria-checked', 'false')
    expect(
      screen.getByText(
        'Your devices on the same network sync directly while Rowel is open and unlocked on both. Nothing leaves your network.'
      )
    ).toBeInTheDocument()
    expect(screen.queryByTestId('settings-nearby-device-d1')).not.toBeInTheDocument()
  })

  it('turns on through the backend, and holds the switch until it reports', async () => {
    report({ enabled: false })
    openNearby()

    await userEvent.click(toggle())

    expect(calls('nearby_set_enabled')).toEqual([{ enabled: true }])
    expect(toggle()).toHaveAttribute('aria-checked', 'true')

    report({ enabled: true })
    expect(toggle()).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByTestId('settings-nearby-searching')).toBeInTheDocument()

    await userEvent.click(toggle())
    expect(calls('nearby_set_enabled')).toEqual([{ enabled: true }, { enabled: false }])
  })

  it('puts the switch back and says why when turning on fails', async () => {
    mockCommand('nearby_set_enabled', () =>
      Promise.reject({ kind: 'other', message: 'the network is unavailable' })
    )
    report({ enabled: false })
    openNearby()

    await userEvent.click(toggle())

    expect(await screen.findByTestId('settings-nearby-error')).toHaveTextContent(
      'the network is unavailable'
    )
    expect(toggle()).toHaveAttribute('aria-checked', 'false')
  })

  it('looks for devices while on with none found', () => {
    report({ devices: [] })
    openNearby()

    const searching = screen.getByTestId('settings-nearby-searching')
    expect(searching).toHaveTextContent('Looking for your devices…')
    expect(searching).toHaveTextContent('Open Rowel on another device on this network.')
  })

  it.each<[string, NearbyDevice['state'], Partial<NearbyDevice>]>([
    ['Syncing…', 'syncing', {}],
    ['Waiting to sync', 'waiting', {}],
    ['Up to date', 'upToDate', {}],
    ['Not nearby', 'offline', {}],
    [
      'Not nearby · last synced 3 days ago',
      'offline',
      { lastSyncedAt: new Date(Date.now() - 3 * DAY - 60_000).toISOString() }
    ],
    ['Paused', 'paused', {}],
    ['Has the new master password', 'passwordChanged', {}],
    ['Needs the new master password', 'needsPassword', {}],
    ['The clocks are 7 minutes apart', 'clockSkew', { detail: 'The clocks are 7 minutes apart' }],
    ['The connection dropped', 'error', { detail: 'The connection dropped' }],
    // A kind is what is said, in the user's language; the sentence is the
    // fallback for one that has nothing to translate.
    [
      'The clocks on these two devices are too far apart to sync safely. Set both to the correct time.',
      'clockSkew',
      { detail: 'this device’s clock and the other device’s are 7 minutes apart', detailKind: 'clockSkew' }
    ],
    [
      'The master password of this lockbox was changed on another device. Enter the new one to keep syncing.',
      'error',
      { detail: 'password changed remotely', detailKind: 'passwordChangedRemotely' }
    ],
    ['the other device broke the protocol', 'error', { detail: 'the other device broke the protocol', detailKind: 'other' }]
  ])('says “%s” for a device in state %s', (line, state, over) => {
    report({ devices: [device({ state, ...over })] })
    openNearby()

    expect(screen.getByTestId('settings-nearby-device-d1')).toHaveTextContent('Alex’s iPhone')
    expect(status()).toHaveTextContent(line)
    expect(status().textContent).toBe(line)
  })

  it('marks the lines that need something done', () => {
    report({
      devices: [
        device({ id: 'ok', state: 'upToDate' }),
        device({ id: 'bad', state: 'needsPassword' })
      ]
    })
    openNearby()

    expect(status('ok')).not.toHaveClass('text-bad')
    expect(status('bad')).toHaveClass('text-bad')
  })

  it('asks for the new master password from the device that has it, and from no other', async () => {
    report({
      devices: [
        device({ id: 'ahead', state: 'passwordChanged' }),
        device({ id: 'behind', state: 'needsPassword' })
      ]
    })
    openNearby()

    expect(screen.queryByTestId('settings-nearby-adopt-behind')).not.toBeInTheDocument()
    await userEvent.click(screen.getByTestId('settings-nearby-adopt-ahead'))
    expect(useUi.getState().passwordPrompt).toBe(true)
  })

  it('pauses a device, and resumes a paused one', async () => {
    report({ devices: [device({ id: 'd1' }), device({ id: 'd2', state: 'paused' })] })
    openNearby()

    await userEvent.click(screen.getByTestId('settings-nearby-menu-d1'))
    expect(screen.getByTestId('settings-nearby-pause-d1')).toHaveTextContent('Pause syncing')
    await userEvent.click(screen.getByTestId('settings-nearby-pause-d1'))

    await userEvent.click(screen.getByTestId('settings-nearby-menu-d2'))
    expect(screen.getByTestId('settings-nearby-pause-d2')).toHaveTextContent('Resume syncing')
    await userEvent.click(screen.getByTestId('settings-nearby-pause-d2'))

    expect(calls('nearby_pause')).toEqual([
      { deviceId: 'd1', paused: true },
      { deviceId: 'd2', paused: false }
    ])
  })

  it('says why a pause failed, on the row', async () => {
    mockCommandOnce('nearby_pause', () =>
      Promise.reject({ kind: 'other', message: 'that device is gone' })
    )
    report({ devices: [device()] })
    openNearby()

    await userEvent.click(screen.getByTestId('settings-nearby-menu-d1'))
    await userEvent.click(screen.getByTestId('settings-nearby-pause-d1'))

    expect(
      await within(screen.getByTestId('settings-nearby-device-d1')).findByText('that device is gone')
    ).toBeInTheDocument()
  })

  it('removes a device only once the confirmation is answered', async () => {
    report({ devices: [device()] })
    openNearby()

    await userEvent.click(screen.getByTestId('settings-nearby-menu-d1'))
    await userEvent.click(screen.getByTestId('settings-nearby-remove-d1'))

    const sheet = screen.getByTestId('settings-nearby-remove-sheet')
    expect(sheet).toHaveTextContent('Remove “Alex’s iPhone”?')
    expect(sheet).toHaveTextContent('This device will no longer sync with it')
    expect(sheet).toHaveTextContent('your other devices still will until you remove it on them too')
    expect(sheet).toHaveTextContent('change your master password in Settings › Security')
    expect(calls('nearby_remove')).toEqual([])

    await userEvent.click(screen.getByTestId('settings-nearby-remove-confirm'))

    expect(calls('nearby_remove')).toEqual([{ deviceId: 'd1' }])
    expect(screen.queryByTestId('settings-nearby-remove-sheet')).not.toBeInTheDocument()
  })

  it('removes nothing when the confirmation is cancelled', async () => {
    report({ devices: [device()] })
    openNearby()

    await userEvent.click(screen.getByTestId('settings-nearby-menu-d1'))
    await userEvent.click(screen.getByTestId('settings-nearby-remove-d1'))
    await userEvent.click(screen.getByTestId('confirm-cancel'))

    expect(screen.queryByTestId('settings-nearby-remove-sheet')).not.toBeInTheDocument()
    expect(calls('nearby_remove')).toEqual([])
  })
})

