import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import App from '@/App'
import WorkspacePicker from '@/components/Auth/WorkspacePicker'
import type { Workspace } from '@/api/types'
import { useApp } from '@/store'
import { appStatusDefault, calls, mockCommand } from './ipc'
import { deferred, resetStores, seedApp } from './utils'
import { setLayout } from './layout'

beforeEach(() => {
  vi.clearAllMocks()
  resetStores()
})

const PRIMARY: Workspace = { id: 'default', name: null }
const WORK: Workspace = { id: 'w2', name: 'Work' }

// What the boot probe said about workspaces, as boot.ts would have stored it —
// and what the probe every switch ends in says again, so the picker is still
// there once it lands.
const seed = (workspaces: Workspace[], activeWorkspace = 'default') => {
  seedApp({ workspaces, activeWorkspace })
  mockCommand('app_status', () => ({ ...appStatusDefault(), workspaces, activeWorkspace }))
}

describe('WorkspacePicker', () => {
  it('draws nothing on a single-workspace install', () => {
    seed([PRIMARY])
    render(<WorkspacePicker />)

    expect(screen.queryByTestId('workspace-picker')).not.toBeInTheDocument()
  })

  // Only the vault about to open is on the screen; the rest wait in the menu.
  it('wears the open workspace as a chip and lists every one in its menu', async () => {
    seed([PRIMARY, WORK])
    render(<WorkspacePicker />)

    expect(screen.getByTestId('workspace-picker')).toBeInTheDocument()
    // The primary has no name of its own until it is given one. The chip is
    // named by the vault it shows, so assistive technology hears which one is
    // about to open rather than a generic label.
    expect(screen.getByTestId('workspace-chip')).toHaveAccessibleName('Personal')
    expect(screen.queryByTestId('workspace-option-w2')).not.toBeInTheDocument()

    await userEvent.click(screen.getByTestId('workspace-chip'))

    // One of several: each row says whether it is the current one.
    expect(screen.getByRole('menuitemradio', { name: /Personal/, checked: true })).toBe(
      screen.getByTestId('workspace-option-default')
    )
    expect(screen.getByRole('menuitemradio', { name: /Work/, checked: false })).toBe(
      screen.getByTestId('workspace-option-w2')
    )

    await userEvent.click(screen.getByTestId('workspace-option-w2'))

    expect(calls('workspace_select')).toEqual([{ id: 'w2' }])
    expect(screen.queryByTestId('workspace-option-w2')).not.toBeInTheDocument()
    // The picked row is gone with the menu; the keyboard lands back on the chip.
    expect(screen.getByTestId('workspace-chip')).toHaveFocus()
  })

  // Where each vault lives is read off what is knowable while it is locked:
  // the open one's live connection, any other's the connection the backend
  // found beside it. A vault id alone is not that — it outlives a disconnect.
  it('says where each workspace lives', async () => {
    seed([PRIMARY, { ...WORK, vaultId: 'v-work', synced: true }, { id: 'w3', name: 'Old', vaultId: 'v-old' }])
    render(<WorkspacePicker />)

    await userEvent.click(screen.getByTestId('workspace-chip'))

    expect(screen.getByTestId('workspace-option-default')).toHaveTextContent('This device')
    expect(screen.getByTestId('workspace-option-w2')).toHaveTextContent('Google Drive')
    expect(screen.getByTestId('workspace-option-w3')).toHaveTextContent('This device')
  })

  // The size its last open here recorded, ahead of where it lives; a vault
  // never opened on this device has no count to give.
  it('says how many items each workspace held when last open', async () => {
    seed([{ ...PRIMARY, itemCount: 284 }, { ...WORK, itemCount: 1 }, { id: 'w3', name: 'New' }])
    render(<WorkspacePicker />)

    await userEvent.click(screen.getByTestId('workspace-chip'))

    expect(screen.getByTestId('workspace-option-default')).toHaveTextContent('284 items · This device')
    expect(screen.getByTestId('workspace-option-w2')).toHaveTextContent('1 item · This device')
    expect(screen.getByTestId('workspace-option-w3')).not.toHaveTextContent('item')
  })

  // A colour the user picked is the tile's, on the chip and in the menu alike;
  // one never picked keeps the hue its id hashes to.
  it('draws a chosen colour as a solid tile', async () => {
    seed([{ ...PRIMARY, color: 'rose' }, WORK])
    render(<WorkspacePicker />)

    expect(screen.getByTestId('workspace-chip').querySelector('.bg-ws-rose')).not.toBeNull()

    await userEvent.click(screen.getByTestId('workspace-chip'))

    expect(screen.getByTestId('workspace-option-default').querySelector('.bg-ws-rose')).not.toBeNull()
    const plain = screen.getByTestId('workspace-option-w2')
    expect(plain.querySelector('.monogram')).not.toBeNull()
    expect(plain.querySelector('[class*="bg-ws-"]')).toBeNull()
  })

  it('closes the menu on Escape and hands focus back to the chip', async () => {
    seed([PRIMARY, WORK])
    render(<WorkspacePicker />)

    await userEvent.click(screen.getByTestId('workspace-chip'))
    expect(screen.getByTestId('workspace-chip')).toHaveAttribute('aria-expanded', 'true')

    await userEvent.keyboard('{Escape}')

    expect(screen.queryByTestId('workspace-option-w2')).not.toBeInTheDocument()
    expect(screen.getByTestId('workspace-chip')).toHaveFocus()
  })

  // Enrollment is per workspace, so the gate on screen mid-switch is the one
  // being left. It stays drawn — taking it down and putting it back blinked
  // the passphrase card — and the switch is flagged so the lock screen takes
  // no attempt against it, until the probe brings the next one's answer.
  it('keeps the gate on screen through a switch, flagged until the probe lands', async () => {
    seedApp({
      workspaces: [PRIMARY, WORK],
      activeWorkspace: 'default',
      biometric: { available: true, canEnroll: true, type: 'touch', mode: 'prompt' }
    })
    const answer = deferred<null>()
    mockCommand('workspace_select', () => answer.promise)
    mockCommand('app_status', () => ({
      ...appStatusDefault(),
      workspaces: [PRIMARY, WORK],
      activeWorkspace: 'w2'
    }))
    render(<WorkspacePicker />)

    await userEvent.click(screen.getByTestId('workspace-chip'))
    await userEvent.click(screen.getByTestId('workspace-option-w2'))

    expect(useApp.getState().switching).toBe(true)
    expect(useApp.getState().status?.biometric.available).toBe(true)
    // The chip still names the vault the gate belongs to.
    expect(screen.getByTestId('workspace-chip')).toHaveAccessibleName('Personal')

    answer.resolve(null)

    await waitFor(() => expect(useApp.getState().switching).toBe(false))
    expect(useApp.getState().status?.activeWorkspace).toBe('w2')
    expect(screen.getByTestId('workspace-chip')).toHaveAccessibleName('Work')
  })

  // An unlock being verified or held on its success beat is opening *this*
  // vault; a switch now would have it enter with the old rows afterwards.
  it('opens nothing and takes no chord while an unlock is in flight', async () => {
    seed([PRIMARY, WORK])
    render(<WorkspacePicker busy />)

    await userEvent.click(screen.getByTestId('workspace-chip'))
    await userEvent.keyboard('{Meta>}2{/Meta}')

    expect(screen.queryByTestId('workspace-option-w2')).not.toBeInTheDocument()
    expect(calls('workspace_select')).toHaveLength(0)
  })

  // A second pick while the first is landing is dropped, not raced: its
  // ending would clear the flag while the first was still in flight.
  it('runs one switch at a time', async () => {
    seed([PRIMARY, WORK, { id: 'w3', name: 'Third' }])
    const answer = deferred<null>()
    mockCommand('workspace_select', () => answer.promise)
    render(<WorkspacePicker />)

    await userEvent.keyboard('{Meta>}2{/Meta}')
    await userEvent.keyboard('{Meta>}3{/Meta}')

    expect(calls('workspace_select')).toEqual([{ id: 'w2' }])
    answer.resolve(null)
    await waitFor(() => expect(useApp.getState().switching).toBe(false))
  })

  it('offers no gate when the probe after a switch cannot be had', async () => {
    seedApp({
      workspaces: [PRIMARY, WORK],
      activeWorkspace: 'default',
      biometric: { available: true, canEnroll: true, type: 'touch', mode: 'prompt' }
    })
    mockCommand('app_status', () => Promise.reject({ kind: 'other', message: 'down' }))
    render(<WorkspacePicker />)

    await userEvent.click(screen.getByTestId('workspace-chip'))
    await userEvent.click(screen.getByTestId('workspace-option-w2'))

    await waitFor(() => expect(useApp.getState().switching).toBe(false))
    expect(useApp.getState().status?.biometric.available).toBe(false)
  })

  it('puts the gate back when the switch is refused', async () => {
    seedApp({
      workspaces: [PRIMARY, WORK],
      biometric: { available: true, canEnroll: true, type: 'touch', mode: 'prompt' }
    })
    mockCommand('app_status', () => ({
      ...appStatusDefault(),
      biometric: { available: true, canEnroll: true, type: 'touch', mode: 'prompt' }
    }))
    mockCommand('workspace_select', () => Promise.reject({ kind: 'other', message: 'busy' }))
    render(<WorkspacePicker />)

    await userEvent.click(screen.getByTestId('workspace-chip'))
    await userEvent.click(screen.getByTestId('workspace-option-w2'))

    await waitFor(() => expect(useApp.getState().status?.biometric.available).toBe(true))
  })

  it('opens the menu on the workspace already open', async () => {
    seed([PRIMARY, WORK], 'w2')
    render(<WorkspacePicker />)

    await userEvent.click(screen.getByTestId('workspace-chip'))

    expect(screen.getByTestId('workspace-option-w2')).toHaveFocus()
  })

  // The chord a row prints is the chord that opens it, menu or no menu.
  it('switches with ⌘ and the workspace’s place on the list', async () => {
    seed([PRIMARY, WORK])
    render(<WorkspacePicker />)

    expect(screen.queryByTestId('workspace-option-w2')).not.toBeInTheDocument()
    await userEvent.keyboard('{Meta>}2{/Meta}')

    expect(calls('workspace_select')).toEqual([{ id: 'w2' }])
  })

  it('prints each other workspace’s chord and checks the open one', async () => {
    seed([PRIMARY, WORK])
    render(<WorkspacePicker />)

    await userEvent.click(screen.getByTestId('workspace-chip'))

    expect(screen.getByTestId('workspace-option-w2')).toHaveTextContent('⌘2')
    expect(screen.getByTestId('workspace-option-default')).not.toHaveTextContent('⌘1')
  })

  it('offers no search while the list is short', async () => {
    seed([PRIMARY, WORK])
    render(<WorkspacePicker />)

    await userEvent.click(screen.getByTestId('workspace-chip'))

    expect(screen.queryByTestId('workspace-search')).not.toBeInTheDocument()
  })

  describe('with enough workspaces to search', () => {
    const MANY: Workspace[] = [
      PRIMARY,
      WORK,
      { id: 'w3', name: 'Northwind' },
      { id: 'w4', name: 'Family' },
      { id: 'w5', name: 'Finance' },
      { id: 'w6', name: 'Clients' }
    ]

    it('opens on the search field and filters the rows by name', async () => {
      seed(MANY)
      render(<WorkspacePicker />)

      await userEvent.click(screen.getByTestId('workspace-chip'))
      expect(screen.getByTestId('workspace-search')).toHaveFocus()

      await userEvent.keyboard('in')

      expect(screen.getAllByRole('option').map(row => row.dataset.testid)).toEqual([
        'workspace-option-w3',
        'workspace-option-w5'
      ])
    })

    // The caret stays in the field: the arrows walk the rows, Enter picks.
    it('picks the row the arrows land on with Enter', async () => {
      seed(MANY)
      render(<WorkspacePicker />)

      await userEvent.click(screen.getByTestId('workspace-chip'))
      await userEvent.keyboard('in{ArrowDown}{Enter}')

      expect(calls('workspace_select')).toEqual([{ id: 'w5' }])
      expect(screen.getByTestId('workspace-chip')).toHaveFocus()
    })

    // The caret never leaves the field, so the field is what says which row
    // the arrows have lit — the one Enter would open.
    it('names the lit row to assistive technology as the arrows move', async () => {
      seed(MANY)
      render(<WorkspacePicker />)

      await userEvent.click(screen.getByTestId('workspace-chip'))
      const search = screen.getByRole('combobox', { name: 'Find a lockbox' })
      const lit = () => document.getElementById(search.getAttribute('aria-activedescendant') ?? '')

      // A combobox's popup is a listbox, its rows options: the lit one is
      // selected, the open vault checked.
      expect(search).toHaveAttribute('aria-controls', screen.getByRole('listbox').id)
      expect(screen.queryByRole('menu')).not.toBeInTheDocument()
      expect(lit()).toBe(screen.getByRole('option', { name: /Personal/, selected: true }))
      expect(lit()).toHaveAttribute('aria-checked', 'true')

      await userEvent.keyboard('{ArrowDown}')
      expect(lit()).toBe(screen.getByRole('option', { name: /Work/, selected: true }))
      expect(screen.getByTestId('workspace-option-default')).toHaveAttribute('aria-selected', 'false')

      await userEvent.keyboard('zzz')
      expect(search).not.toHaveAttribute('aria-activedescendant')
    })

    it('says so when nothing matches', async () => {
      seed(MANY)
      render(<WorkspacePicker />)

      await userEvent.click(screen.getByTestId('workspace-chip'))
      await userEvent.keyboard('zzz')

      expect(screen.queryAllByRole('option')).toHaveLength(0)
      expect(screen.getByTestId('workspace-search-empty')).toHaveTextContent('zzz')
    })
  })

  it('does nothing when the workspace already open is picked', async () => {
    seed([PRIMARY, WORK])
    render(<WorkspacePicker />)

    // Switching to it would lock and re-open the very screen it is on.
    await userEvent.click(screen.getByTestId('workspace-chip'))
    await userEvent.click(screen.getByTestId('workspace-option-default'))

    expect(calls('workspace_select')).toHaveLength(0)
    expect(screen.queryByTestId('workspace-option-default')).not.toBeInTheDocument()
    // The row that was picked is gone with the menu; the keyboard is back on
    // the chip rather than dropped on the body.
    expect(screen.getByTestId('workspace-chip')).toHaveFocus()
  })
})

// The phone's lock screen mounts the same picker, and through `App` so the
// switch runs exactly as it does on a device.
describe('WorkspacePicker on the phone lock screen', () => {
  const GATE = { available: true, canEnroll: true, type: 'face', mode: 'prompt' } as const

  beforeEach(() => setLayout('compact'))

  it('lists each workspace with its size and where it lives', async () => {
    seed([{ ...PRIMARY, itemCount: 284 }, { ...WORK, itemCount: 12 }])
    render(<App />)

    await userEvent.click(await screen.findByTestId('workspace-chip'))

    expect(screen.getByTestId('workspace-option-default')).toHaveTextContent('284 items · This device')
    expect(screen.getByTestId('workspace-option-w2')).toHaveTextContent('12 items · This device')
  })

  // The tile is the phone's whole lead: taking the gate down for the round
  // trip swapped it for the passphrase card and back.
  it('keeps the biometric tile on screen through a switch', async () => {
    seedApp({ workspaces: [PRIMARY, WORK], activeWorkspace: 'default', biometric: GATE })
    const answer = deferred<null>()
    mockCommand('workspace_select', () => answer.promise)
    mockCommand('app_status', () => ({
      ...appStatusDefault(),
      workspaces: [PRIMARY, WORK],
      activeWorkspace: 'w2',
      biometric: GATE
    }))
    render(<App />)

    await userEvent.click(await screen.findByTestId('workspace-chip'))
    await userEvent.click(screen.getByTestId('workspace-option-w2'))

    expect(useApp.getState().switching).toBe(true)
    expect(screen.getByTestId('biometric-tile')).toBeInTheDocument()
    expect(screen.queryByTestId('unlock-password-input')).not.toBeInTheDocument()

    answer.resolve(null)

    await waitFor(() => expect(screen.getByTestId('workspace-chip')).toHaveAccessibleName('Work'))
    expect(screen.getByTestId('biometric-tile')).toBeInTheDocument()
  })
})
