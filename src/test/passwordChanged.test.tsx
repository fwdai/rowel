import { describe, it, expect } from 'vitest'
import { act, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Header from '@/components/Main/Header'
import Settings from '@/components/Main/Sidebar/Settings'
import PasswordChanged from '@/components/Main/PasswordChanged'
import type { SyncStatus } from '@/api/sync'
import { initialApp, openPasswordPrompt, setSyncStatus, useUi } from '@/store'
import { calls, mockCommandOnce } from './ipc'
import { seedApp } from './utils'

const report = (s: Partial<SyncStatus>) => setSyncStatus({ ...initialApp.sync, ...s })

const changedElsewhere = () =>
  report({
    configured: true,
    error: 'the master password of this lockbox was changed on another device',
    errorKind: 'passwordChangedRemotely'
  })

const dialog = () => screen.queryByTestId('password-changed-modal')
const field = () => screen.getByTestId('password-changed-password')

const showPrompt = () => {
  render(<PasswordChanged />)
  act(openPasswordPrompt)
}

const submit = async (password: string) => {
  await userEvent.type(field(), password)
  await userEvent.click(screen.getByTestId('password-changed-submit'))
}

describe('Master password changed on another device', () => {
  it('stays down until something raises it', () => {
    render(<PasswordChanged />)
    expect(dialog()).not.toBeInTheDocument()
  })

  it('asks for the new password and submits it', async () => {
    showPrompt()

    expect(dialog()).toHaveTextContent('Master password changed on another device')
    expect(dialog()).toHaveTextContent(
      'Enter the new master password to keep this lockbox in sync. This device will switch to it.'
    )
    await submit('new horse battery')

    expect(calls('sync_adopt_password')).toEqual([{ password: 'new horse battery' }])
    // Everything moved with it: nothing left to say.
    expect(dialog()).not.toBeInTheDocument()
    expect(useUi.getState().passwordPrompt).toBe(false)
  })

  // The pack may not be one this user re-keyed at all; the copy says so and
  // points at the one way out that does not need the password.
  it('explains a wrong password and clears the field', async () => {
    mockCommandOnce('sync_adopt_password', () =>
      Promise.reject({ kind: 'invalidPassword', message: 'invalid password' })
    )
    showPrompt()

    await submit('typo')

    expect(await screen.findByTestId('password-changed-error')).toHaveTextContent(
      "That isn't the new password. If you didn't change it on another device, the account's copy may not be yours; you can replace it from Settings › Lockboxes."
    )
    expect(field()).toHaveValue('')
    expect(dialog()).toBeInTheDocument()
  })

  it('reports the unlock backoff once attempts escalate', async () => {
    mockCommandOnce('sync_adopt_password', () =>
      Promise.reject({ kind: 'tooManyAttempts', message: 'too many attempts', retryAfterSecs: 8 })
    )
    showPrompt()

    await submit('typo')

    expect(await screen.findByTestId('password-changed-error')).toHaveTextContent(
      'Too many failed attempts. Try again in 8s'
    )
    expect(field()).toBeDisabled()
  })

  // A lockbox beside this one on a password of its own did not move; the user
  // hears which, by the name Settings › Lockboxes shows it under.
  it('names the lockboxes that kept their own password', async () => {
    seedApp({
      workspaces: [
        { id: 'default', name: null },
        { id: 'w2', name: 'Work' },
        { id: 'w3', name: 'Family' }
      ],
      activeWorkspace: 'default'
    })
    mockCommandOnce('sync_adopt_password', () => ['w2'])
    showPrompt()

    await submit('new horse battery')

    const unmoved = await screen.findByTestId('password-changed-unmoved')
    expect(unmoved).toHaveTextContent(
      'These lockboxes use a different password and will ask for it when opened:'
    )
    expect(unmoved).toHaveTextContent('Work')
    expect(unmoved).not.toHaveTextContent('Family')

    await userEvent.click(screen.getByTestId('password-changed-close'))
    expect(dialog()).not.toBeInTheDocument()
  })

  it('closes on Not now without asking the backend', async () => {
    showPrompt()

    await userEvent.type(field(), 'half typed')
    await userEvent.click(screen.getByTestId('password-changed-cancel'))

    expect(dialog()).not.toBeInTheDocument()
    expect(calls('sync_adopt_password')).toHaveLength(0)
  })

  it('opens from the Settings › Lockboxes row', async () => {
    const { container } = render(
      <>
        <Settings />
        <PasswordChanged />
      </>
    )
    await userEvent.click(container.querySelector('.settings-button')!)
    await userEvent.click(screen.getByTestId('settings-nav-workspaces'))
    await act(async () => changedElsewhere())

    await userEvent.click(screen.getByTestId('workspace-adopt-password'))

    expect(dialog()).toBeInTheDocument()
  })

  // The chip's failure state is where this is first noticed; it goes straight
  // to the prompt rather than to Settings.
  it('opens from the header sync chip', async () => {
    changedElsewhere()
    render(
      <>
        <Header />
        <PasswordChanged />
      </>
    )

    await userEvent.click(screen.getByTestId('sync-indicator'))

    expect(dialog()).toBeInTheDocument()
    expect(useUi.getState().settings).toBe(false)
  })
})
