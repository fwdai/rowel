import { describe, it, expect, vi } from 'vitest'
import { act, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { EVENTS } from '@/api/events'
import { ASSOCIATE_TIMEOUT_MS, type PasskeyAsk } from '@/api/browser'
import BrowserConsent from '@/components/Main/BrowserConsent'
import { useUi } from '@/store'
import { clearSession } from '@/store/app'
import { subscribeToEvents } from '@/store/events'
import { calls } from './ipc'
import { emitEvent } from './events'

const REGISTER: PasskeyAsk = {
  id: 'ask-1',
  kind: 'register',
  rpId: 'example.com',
  origin: 'https://login.example.com',
  userName: 'alice',
  userDisplayName: 'Alice Example'
}

const GET: PasskeyAsk = {
  id: 'ask-2',
  kind: 'get',
  rpId: 'github.com',
  origin: 'https://github.com',
  accounts: [{ userName: 'octocat', userDisplayName: 'The Octocat' }]
}

const GET_SEVERAL: PasskeyAsk = {
  ...GET,
  id: 'ask-3',
  accounts: [
    { userName: 'work', userDisplayName: 'Octocat at work' },
    { userName: '', userDisplayName: 'Octocat at home' }
  ]
}

describe('the passkey consent dialog', () => {
  const ask = async (payload: PasskeyAsk = REGISTER) => {
    subscribeToEvents()
    render(<BrowserConsent />)
    act(() => emitEvent(EVENTS.browserPasskey, payload))
    return screen.findByTestId('browser-passkey-modal')
  }

  it('names the site, the account and the page, and sends a yes on Allow', async () => {
    const modal = await ask()
    expect(screen.getByTestId('browser-passkey-title')).toHaveTextContent(
      'example.com wants to create a passkey for alice'
    )
    expect(modal).toHaveTextContent('https://login.example.com')

    await userEvent.click(screen.getByTestId('browser-passkey-allow'))

    expect(calls('browser_passkey_respond')).toEqual([{ id: 'ask-1', allow: true }])
    expect(screen.queryByTestId('browser-passkey-modal')).not.toBeInTheDocument()
    expect(useUi.getState().consentAsk).toBeNull()
  })

  it('asks to sign in as the one account, and sends a no on Deny', async () => {
    await ask(GET)
    expect(screen.getByTestId('browser-passkey-title')).toHaveTextContent(
      'github.com wants to sign in with your passkey for octocat'
    )
    expect(screen.queryByTestId('browser-passkey-account-0')).not.toBeInTheDocument()

    await userEvent.click(screen.getByTestId('browser-passkey-deny'))

    expect(calls('browser_passkey_respond')).toEqual([{ id: 'ask-2', allow: false }])
    expect(screen.queryByTestId('browser-passkey-modal')).not.toBeInTheDocument()
  })

  it('offers the accounts of a sign-in, the newest picked, and sends the pick', async () => {
    await ask(GET_SEVERAL)
    expect(screen.getByTestId('browser-passkey-title')).toHaveTextContent(
      /^github\.com wants to sign in with your passkey$/
    )
    expect(screen.getByTestId('browser-passkey-account-0')).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByTestId('browser-passkey-account-0')).toHaveTextContent('work')
    // An account the site gave no name falls back to its display name.
    expect(screen.getByTestId('browser-passkey-account-1')).toHaveTextContent('Octocat at home')

    await userEvent.click(screen.getByTestId('browser-passkey-account-1'))
    await userEvent.click(screen.getByTestId('browser-passkey-allow'))

    expect(calls('browser_passkey_respond')).toEqual([{ id: 'ask-3', allow: true, account: 1 }])
  })

  it('tells two passkeys named alike apart by when they were added', async () => {
    const twin = { userName: 'octocat', userDisplayName: 'The Octocat' }
    const now = Date.now()
    await ask({
      ...GET,
      id: 'ask-4',
      accounts: [
        { ...twin, createdAt: new Date(now - 2 * 60_000).toISOString() },
        { ...twin, createdAt: new Date(now - 3 * 24 * 60 * 60_000).toISOString() }
      ]
    })
    const [first, second] = ['0', '1'].map(i =>
      screen.getByTestId(`browser-passkey-account-${i}`)
    )
    expect(first).toHaveTextContent('octocat (The Octocat)')
    expect(second).toHaveTextContent('octocat (The Octocat)')
    expect(first).toHaveTextContent('2 minutes ago')
    expect(second).toHaveTextContent('3 days ago')
  })

  it('falls back to the display name, then to no account at all', async () => {
    await ask({ ...REGISTER, userName: '' })
    expect(screen.getByTestId('browser-passkey-title')).toHaveTextContent(
      'example.com wants to create a passkey for Alice Example'
    )
    act(() =>
      emitEvent(EVENTS.browserPasskey, { ...REGISTER, userName: undefined, userDisplayName: '' })
    )
    expect(screen.getByTestId('browser-passkey-title')).toHaveTextContent(
      /^example\.com wants to create a passkey$/
    )
  })

  it('replaces an ask still on screen with a newer one, and answers for that one', async () => {
    await ask()
    act(() => emitEvent(EVENTS.browserPasskey, GET))

    expect(screen.getByTestId('browser-passkey-title')).toHaveTextContent(
      'github.com wants to sign in with your passkey'
    )
    await userEvent.click(screen.getByTestId('browser-passkey-allow'))
    expect(calls('browser_passkey_respond')).toEqual([{ id: 'ask-2', allow: true }])
  })

  it('shows one security decision at a time, whichever kind', async () => {
    await ask(GET)
    // An extension asking to connect takes the place of the passkey dialog
    // left up, rather than opening beside it.
    act(() => emitEvent(EVENTS.browserAssociate, { id: 'ask-9', key: 'a'.repeat(43) + '=' }))

    expect(screen.queryByTestId('browser-passkey-modal')).not.toBeInTheDocument()
    expect(screen.getByTestId('browser-associate-modal')).toBeInTheDocument()
    expect(useUi.getState().consentAsk?.kind).toBe('associate')
  })

  it('leaves on its own once Rust has given up on the ask', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    try {
      await ask()
      act(() => {
        vi.advanceTimersByTime(ASSOCIATE_TIMEOUT_MS)
      })
      expect(screen.queryByTestId('browser-passkey-modal')).not.toBeInTheDocument()
      expect(calls('browser_passkey_respond')).toEqual([])
    } finally {
      vi.useRealTimers()
    }
  })

  it('is gone once the vault locks', async () => {
    await ask()
    act(() => clearSession())

    expect(screen.queryByTestId('browser-passkey-modal')).not.toBeInTheDocument()
    expect(calls('browser_passkey_respond')).toEqual([])
  })
})
