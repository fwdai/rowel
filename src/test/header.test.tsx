import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Header from '@/components/Main/Header'
import type { SyncStatus } from '@/api/sync'
import { initialApp, setNearbyStatus, setSyncStatus, useUi } from '@/store'
import { nearbyUnavailable } from './ipc'

beforeEach(() => vi.clearAllMocks())

const report = (s: Partial<SyncStatus>) => setSyncStatus({ ...initialApp.sync, ...s })

// Sync configured *and* one run already landed -- the pair the chip needs
// before it will claim to be up to date.
const synced = (s: Partial<SyncStatus> = {}) =>
  report({ configured: true, lastSyncedAt: '2024-01-01T00:00:00.000Z', ...s })

const chip = () => screen.getByTestId('sync-indicator')

describe('Header', () => {
  it('reports a local-only vault rather than hiding the chip', () => {
    render(<Header />)
    expect(chip()).toHaveAttribute('data-tone', 'local')
    expect(chip()).toHaveAccessibleName(/this device only/i)
  })

  // The bar is a status line: the chip is its only control. Lock moved to the
  // foot of the rail (see rail.test).
  it('holds nothing but the sync chip', () => {
    render(<Header />)
    expect(screen.getAllByRole('button')).toEqual([chip()])
  })

  it('stays unbadged on a fresh connection, before any run has landed', () => {
    report({ configured: true })
    render(<Header />)
    // Not 'good': no error is the default, and a chip that ticked here would be
    // vouching for a sync that has not happened.
    expect(chip()).toHaveAttribute('data-tone', 'idle')
  })

  it('marks a landed sync as successful', () => {
    synced()
    render(<Header />)
    expect(chip()).toHaveAttribute('data-tone', 'good')
  })

  it('spins while a sync is in flight', () => {
    synced({ inProgress: true })
    render(<Header />)
    expect(chip()).toHaveAttribute('data-tone', 'loading')
  })

  it('surfaces the backend message on a failed sync', () => {
    synced({ error: 'Drive said no' })
    render(<Header />)
    expect(chip()).toHaveAttribute('data-tone', 'bad')
    expect(chip()).toHaveAccessibleName('Drive said no')
  })

  it('reads as syncing, not failed, when a retry follows an error', () => {
    synced({ error: 'Drive said no' })
    synced({ inProgress: true })
    render(<Header />)
    expect(chip()).toHaveAttribute('data-tone', 'loading')
  })

  // Nearby sync makes "this device only" untrue, so it says so where no cloud
  // account speaks for the vault.
  it('speaks for nearby sync when no cloud account is connected', () => {
    setNearbyStatus({
      ...nearbyUnavailable(),
      available: true,
      enabled: true,
      devices: [
        {
          id: 'd1',
          name: 'Alex’s iPhone',
          platform: 'ios',
          state: 'upToDate',
          lastSyncedAt: null,
          detail: null,
          detailKind: null
        }
      ],
      seq: 1
    })
    render(<Header />)
    expect(chip()).toHaveAttribute('data-tone', 'good')
    expect(chip()).toHaveAccessibleName('Up to date with nearby devices')

    // A connected account takes the chip back.
    act(() => synced())
    expect(chip()).toHaveAccessibleName('Sync Successful')
  })

  it('opens Settings › Sync & backup from the sync chip', async () => {
    synced()
    render(<Header />)

    await userEvent.click(chip())
    expect(useUi.getState().settings).toBe(true)
    expect(useUi.getState().settingsSection).toBe('sync')
  })
})
