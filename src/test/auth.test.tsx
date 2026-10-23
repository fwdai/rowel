import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Auth from '@/components/Auth'
import { useApp, flowSetup } from '@/store'
import type { Workspace } from '@/api/types'
import { calls, mockCommand, mockCommandOnce } from './ipc'
import { seedApp } from './utils'

const PRIMARY: Workspace = { id: 'default', name: null }
const WORK: Workspace = { id: 'w2', name: 'Work' }

beforeEach(() => vi.clearAllMocks())

describe('Auth', () => {
  it('renders the lock screen', () => {
    render(<Auth biometric={false} />)
    expect(screen.getByPlaceholderText('Master Password')).toBeInTheDocument()
  })

  // The footer is chrome: the bare version, and where the vault lives as a
  // glyph whose sentence is its accessible name (and its tooltip).
  it('footers the version and where the vault lives', async () => {
    render(<Auth biometric={false} />)
    expect(await screen.findByTestId('app-version')).toHaveTextContent('1.0.0')
    expect(screen.queryByText(/Rowel/)).not.toBeInTheDocument()
    expect(screen.getByLabelText('Stored on this device')).toHaveAttribute('data-state', 'off')
  })

  it('shows Drive in the footer once the vault syncs', async () => {
    useApp.setState(state => ({
      sync: { ...state.sync, configured: true, lastSyncedAt: new Date().toISOString() }
    }))
    render(<Auth biometric={false} />)
    const indicator = await screen.findByTestId('sync-indicator')
    expect(indicator).toHaveAttribute('data-state', 'on')
    expect(indicator).toHaveAccessibleName('Synced with Google Drive. Last synced just now')
  })

  it('has no unlock button — Enter is the only way to submit', () => {
    render(<Auth biometric />)
    expect(screen.queryByLabelText('Unseal')).not.toBeInTheDocument()
  })

  it('keeps Touch ID in the field and reveals the eye only once typing starts', async () => {
    render(<Auth biometric />)

    expect(screen.getByLabelText('Touch ID')).toBeInTheDocument()
    expect(screen.queryByLabelText('Reveal passphrase')).not.toBeInTheDocument()

    await userEvent.type(screen.getByPlaceholderText('Master Password'), 'a')

    expect(screen.getByLabelText('Reveal passphrase')).toBeInTheDocument()
  })

  it('puts the caret where you click so backspace edits mid-passphrase', async () => {
    render(<Auth biometric={false} />)
    const input = screen.getByPlaceholderText<HTMLInputElement>('Master Password')

    await userEvent.type(input, 'abcd')
    expect(input.selectionStart).toBe(4)

    // jsdom lays the cell row out at x=0, so 30px (two 15px cells) is the
    // boundary between 'b' and 'c'.
    fireEvent.mouseDown(input, { clientX: 30, button: 0 })
    expect(input.selectionStart).toBe(2)
    expect(input.selectionEnd).toBe(2)

    await userEvent.keyboard('{Backspace}')
    expect(input.value).toBe('acd')
    expect(input.selectionStart).toBe(1)
  })

  it('acknowledges Enter immediately with a verifying state', async () => {
    // Never resolves: we're asserting the in-flight presentation.
    mockCommand('unlock', () => new Promise(() => {}))
    render(<Auth biometric={false} />)

    await userEvent.type(screen.getByPlaceholderText('Master Password'), 'pw{Enter}')

    expect(screen.getByTestId('unlock-status')).toHaveTextContent('Verifying')
    expect(screen.getByTestId('lock-mascot')).toHaveAttribute('data-state', 'checking')
    expect(screen.getByPlaceholderText('Master Password')).toBeDisabled()
  })

  it('walks the mascot through idle → typing → error → success', async () => {
    mockCommandOnce('unlock', () =>
      Promise.reject({ kind: 'invalidPassword', message: 'invalid master password' })
    )
    mockCommandOnce('unlock', () => ({ entries: [], syncConfigured: false }))
    render(<Auth biometric={false} />)
    const mascot = () => screen.getByTestId('lock-mascot')
    const input = screen.getByPlaceholderText('Master Password')

    expect(mascot()).toHaveAttribute('data-state', 'idle')

    await userEvent.type(input, 'bad')
    expect(mascot()).toHaveAttribute('data-state', 'typing')

    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(mascot()).toHaveAttribute('data-state', 'error'))

    await userEvent.clear(input)
    await userEvent.type(input, 'right{Enter}')
    await waitFor(() =>
      expect(mascot()).toHaveAttribute('data-state', 'success')
    )
    // The vault entry is held back briefly so the celebration can play.
    expect(useApp.getState().flow).not.toBe('main')
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
  })

  // The wide and compact lock screens are different components, so a resize
  // across the layout breakpoint during the celebration hold remounts the hook.
  // Rust has already opened the vault by then; the UI must follow it in.
  it('still enters the vault when unmounted during the success hold', async () => {
    mockCommand('unlock', () => ({ entries: [], syncConfigured: false }))
    const { unmount } = render(<Auth biometric={false} />)

    await userEvent.type(screen.getByPlaceholderText('Master Password'), 'right{Enter}')
    await waitFor(() =>
      expect(screen.getByTestId('lock-mascot')).toHaveAttribute('data-state', 'success')
    )
    expect(useApp.getState().flow).not.toBe('main')

    unmount()

    expect(useApp.getState().flow).toBe('main')
  })

  // ...but not when the unmount is because the flow itself moved on: that
  // result describes a session someone else has since replaced.
  it('drops a held unlock if the flow has already left the lock screen', async () => {
    mockCommand('unlock', () => ({ entries: [], syncConfigured: false }))
    const { unmount } = render(<Auth biometric={false} />)

    await userEvent.type(screen.getByPlaceholderText('Master Password'), 'right{Enter}')
    await waitFor(() =>
      expect(screen.getByTestId('lock-mascot')).toHaveAttribute('data-state', 'success')
    )

    flowSetup()
    unmount()

    expect(useApp.getState().flow).toBe('setup')
  })

  it('unlocks the vault on Enter', async () => {
    mockCommand('unlock', () => ({ entries: [], syncConfigured: false }))
    render(<Auth biometric={false} />)

    await userEvent.type(screen.getByPlaceholderText('Master Password'), 'hunter2{Enter}')

    expect(calls('unlock')).toContainEqual({ password: 'hunter2' })
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
  })

  it('shows an error on a wrong password', async () => {
    mockCommand('unlock', () => Promise.reject({ kind: 'invalidPassword', message: 'invalid master password' }))
    render(<Auth biometric={false} />)

    await userEvent.type(screen.getByPlaceholderText('Master Password'), 'bad{Enter}')

    expect(await screen.findByText('Incorrect Master Password')).toBeInTheDocument()
  })

  it('names the real problem when the vault schema is newer than the app', async () => {
    mockCommand('unlock', () =>
      Promise.reject({ kind: 'vaultTooNew', message: 'vault requires a newer version of the app' })
    )
    render(<Auth biometric={false} />)

    await userEvent.type(screen.getByPlaceholderText('Master Password'), 'right{Enter}')

    expect(await screen.findByText('Vault needs a newer version of Rowel')).toBeInTheDocument()
    expect(screen.queryByText('Incorrect Master Password')).not.toBeInTheDocument()
  })

  // The backend keeps I/O and corruption failures out of `invalidPassword` (and
  // out of the lockout count); the screen must not fold them back in.
  it('shows the real cause when the vault fails to open for a non-password reason', async () => {
    mockCommand('unlock', () =>
      Promise.reject({ kind: 'other', message: 'could not open the vault: io: disk I/O error' })
    )
    render(<Auth biometric={false} />)

    await userEvent.type(screen.getByPlaceholderText('Master Password'), 'right{Enter}')

    expect(await screen.findByTestId('unlock-error')).toHaveTextContent(
      'could not open the vault: io: disk I/O error'
    )
    expect(screen.queryByText('Incorrect Master Password')).not.toBeInTheDocument()
  })

  it('disables the input and shows a countdown on too many attempts', async () => {
    mockCommand('unlock', () =>
      Promise.reject({ kind: 'tooManyAttempts', message: 'too many attempts', retryAfterSecs: 2 })
    )
    render(<Auth biometric={false} />)

    await userEvent.type(screen.getByPlaceholderText('Master Password'), 'bad{Enter}')

    expect(await screen.findByText(/Try again in 2s/)).toBeInTheDocument()
    expect(screen.getByPlaceholderText('Master Password')).toBeDisabled()
  })

  it('unlocks with biometrics', async () => {
    mockCommand('unlock_biometric', () => ({ entries: [], syncConfigured: false }))
    render(<Auth biometric />)

    await userEvent.click(screen.getByLabelText('Touch ID'))

    expect(calls('unlock_biometric')).toHaveLength(1)
    await waitFor(() => expect(useApp.getState().flow).toBe('main'))
  })

  // Mid-switch the backend already points at the next vault while the screen
  // still shows the last one, so nothing typed or touched is sent anywhere.
  it('takes no attempt while a workspace switch is in flight', async () => {
    useApp.setState({ switching: true })
    try {
      render(<Auth biometric />)

      await userEvent.click(screen.getByLabelText('Touch ID'))
      await userEvent.type(screen.getByPlaceholderText('Master Password'), 'secret{Enter}')

      expect(calls('unlock_biometric')).toHaveLength(0)
      expect(calls('unlock')).toHaveLength(0)
    } finally {
      useApp.setState({ switching: false })
    }
  })

  // A lockbox that opened with a password of its own stays sealed: the card
  // asks for the vault password next, moves the lockbox onto it, and opens it
  // with it.
  describe('a lockbox on a password of its own', () => {
    const offPassword = () =>
      mockCommandOnce('unlock', () =>
        Promise.reject({
          kind: 'vaultPasswordRequired',
          message: 'this lockbox has a different password; enter the vault password to move it over',
          workspaceId: 'w2'
        })
      )

    it('asks for the vault password, moves the lockbox and opens it', async () => {
      seedApp({ workspaces: [PRIMARY, WORK], activeWorkspace: 'w2' })
      offPassword()
      render(<Auth biometric={false} />)

      await userEvent.type(screen.getByPlaceholderText('Master Password'), 'its-own{Enter}')

      expect(await screen.findByTestId('unlock-status')).toHaveTextContent(
        'Rowel opens every lockbox with your vault password. Enter it to move this lockbox over.'
      )
      const field = screen.getByPlaceholderText('Vault password')
      expect(field).toHaveValue('')

      await userEvent.type(field, 'the-vault{Enter}')

      expect(calls('workspace_adopt_vault_password')).toEqual([
        { workspaceId: 'w2', lockboxPassword: 'its-own', vaultPassword: 'the-vault' }
      ])
      await waitFor(() =>
        expect(calls('unlock')).toEqual([{ password: 'its-own' }, { password: 'the-vault' }])
      )
      await waitFor(() => expect(useApp.getState().flow).toBe('main'))
    })

    it('keeps asking for the vault password after a wrong one', async () => {
      seedApp({ workspaces: [PRIMARY, WORK], activeWorkspace: 'w2' })
      offPassword()
      mockCommandOnce('workspace_adopt_vault_password', () =>
        Promise.reject({ kind: 'invalidPassword', message: 'invalid master password' })
      )
      render(<Auth biometric={false} />)

      await userEvent.type(screen.getByPlaceholderText('Master Password'), 'its-own{Enter}')
      await userEvent.type(await screen.findByPlaceholderText('Vault password'), 'nope{Enter}')

      expect(await screen.findByText('Incorrect Master Password')).toBeInTheDocument()
      expect(screen.getByPlaceholderText('Vault password')).toBeInTheDocument()
      expect(calls('unlock')).toHaveLength(1)
    })
  })

  // A passphrase typed for one vault must not be the one submitted to the next.
  it('starts the next vault on an empty field', async () => {
    seedApp({ workspaces: [PRIMARY, WORK], activeWorkspace: 'default' })
    render(<Auth biometric={false} />)
    const field = screen.getByPlaceholderText('Master Password')
    await userEvent.type(field, 'for-personal')

    act(() => seedApp({ workspaces: [PRIMARY, WORK], activeWorkspace: 'w2' }))

    expect(screen.getByPlaceholderText('Master Password')).toHaveValue('')
  })
})
