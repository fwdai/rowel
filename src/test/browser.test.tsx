import { describe, it, expect, vi } from 'vitest'
import { act, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { EVENTS } from '@/api/events'
import { ASSOCIATE_TIMEOUT_MS, type BrowserStatus } from '@/api/browser'
import Settings from '@/components/Main/Sidebar/Settings'
import BrowserConsent from '@/components/Main/BrowserConsent'
import { openSettings, useUi } from '@/store'
import { subscribeToEvents } from '@/store/events'
import { calls, mockCommand } from './ipc'
import { emitEvent } from './events'

const KEY = 'AAAAbbbbCCCCddddEEEEffffGGGGhhhhIIIIjjjjKKK='

const status = (overrides: Partial<BrowserStatus> = {}): BrowserStatus => ({
  enabled: false,
  browsers: [
    { id: 'chrome', label: 'Google Chrome', detected: true, installed: false, conflict: false },
    { id: 'edge', label: 'Microsoft Edge', detected: true, installed: false, conflict: true },
    { id: 'firefox', label: 'Firefox', detected: false, installed: false, conflict: false }
  ],
  clients: [],
  ...overrides
})

const openSection = async () => {
  openSettings('browser')
  render(<Settings />)
  return screen.findByTestId('settings-browser-chrome')
}

describe('Settings › Browser extension', () => {
  it('shows what Rust reports and turns the host on', async () => {
    mockCommand('browser_status', () => status())
    mockCommand('browser_set_enabled', () =>
      status({
        enabled: true,
        browsers: [
          { id: 'chrome', label: 'Google Chrome', detected: true, installed: true, conflict: false }
        ]
      })
    )
    const chrome = await openSection()

    expect(chrome).toHaveTextContent('Detected')
    expect(screen.getByTestId('settings-browser-edge')).toHaveTextContent('Registered to KeePassXC')
    expect(screen.getByTestId('settings-browser-firefox')).toHaveTextContent('Not found')
    expect(screen.getByTestId('settings-browser-clients-empty')).toBeInTheDocument()

    await userEvent.click(screen.getByTestId('settings-browser-toggle'))

    expect(calls('browser_set_enabled')).toEqual([{ enabled: true }])
    expect(screen.getByTestId('settings-browser-toggle')).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByTestId('settings-browser-chrome')).toHaveTextContent('Ready')
  })

  it('forgets a connected extension', async () => {
    mockCommand('browser_status', () => status({ clients: [{ name: 'Work Chrome', key: KEY }] }))
    await openSection()
    const row = screen.getByTestId('settings-browser-client')
    expect(row).toHaveTextContent('Work Chrome')
    expect(row).toHaveTextContent('AAAAbbbb…KKK=')

    await userEvent.click(within(row).getByTestId('settings-browser-forget'))

    expect(calls('browser_forget_client')).toEqual([{ key: KEY }])
    expect(screen.queryByTestId('settings-browser-client')).not.toBeInTheDocument()
  })

  it('takes one change at a time', async () => {
    let settle: (status: BrowserStatus) => void = () => {}
    mockCommand('browser_status', () =>
      status({
        clients: [
          { name: 'Work Chrome', key: KEY },
          { name: 'Home Firefox', key: 'ZZZZyyyyXXXXwwww0000=' }
        ]
      })
    )
    mockCommand('browser_forget_client', () => new Promise(resolve => (settle = resolve)))
    await openSection()
    const [first, second] = screen.getAllByTestId('settings-browser-forget')

    await userEvent.click(first)

    // The first forget is in flight: nothing else can be asked for.
    expect(second).toBeDisabled()
    expect(screen.getByTestId('settings-browser-toggle')).toBeDisabled()
    await userEvent.click(second)
    expect(calls('browser_forget_client')).toEqual([{ key: KEY }])

    await act(async () => settle(status({ clients: [{ name: 'Home Firefox', key: 'Z' }] })))
    expect(screen.getByTestId('settings-browser-forget')).toBeEnabled()
  })

  it('re-reads its list when the consent dialog lets an extension in', async () => {
    mockCommand('browser_status', () => status())
    subscribeToEvents()
    await openSection()
    expect(screen.getByTestId('settings-browser-clients-empty')).toBeInTheDocument()

    mockCommand('browser_status', () => status({ clients: [{ name: 'Work Chrome', key: KEY }] }))
    act(() => emitEvent(EVENTS.browserClients, undefined))

    expect(await screen.findByTestId('settings-browser-client')).toHaveTextContent('Work Chrome')
  })

  it('holds a refresh until the change in flight has landed', async () => {
    let settle: (status: BrowserStatus) => void = () => {}
    mockCommand('browser_status', () => status({ clients: [{ name: 'Work Chrome', key: KEY }] }))
    mockCommand('browser_forget_client', () => new Promise(resolve => (settle = resolve)))
    subscribeToEvents()
    await openSection()
    await userEvent.click(screen.getByTestId('settings-browser-forget'))

    // Let in while the forget is out: not read yet, so the forget's own
    // answer is not thrown away as stale.
    mockCommand('browser_status', () => status({ clients: [{ name: 'Home Firefox', key: 'Z' }] }))
    act(() => emitEvent(EVENTS.browserClients, undefined))
    expect(calls('browser_status')).toHaveLength(1)

    await act(async () => settle(status({ clients: [] })))
    expect(calls('browser_status')).toHaveLength(2)
    expect(await screen.findByTestId('settings-browser-client')).toHaveTextContent('Home Firefox')
  })
})

describe('the browser consent dialog', () => {
  const ask = async (key = KEY, id = 'ask-1') => {
    subscribeToEvents()
    render(<BrowserConsent />)
    act(() => emitEvent(EVENTS.browserAssociate, { id, key }))
    return screen.findByTestId('browser-associate-modal')
  }

  it('opens on the ask and sends the name it was given', async () => {
    await ask()
    expect(screen.getByTestId('browser-associate-fingerprint')).toHaveTextContent('AAAAbbbb…KKK=')
    const name = screen.getByTestId('browser-associate-name')
    expect(name).toHaveValue('Browser')

    await userEvent.clear(name)
    await userEvent.type(name, 'Work laptop')
    await userEvent.click(screen.getByTestId('browser-associate-allow'))

    expect(calls('browser_respond')).toEqual([{ id: 'ask-1', name: 'Work laptop' }])
    expect(screen.queryByTestId('browser-associate-modal')).not.toBeInTheDocument()
    expect(useUi.getState().consentAsk).toBeNull()
  })

  it('sends null on Deny', async () => {
    await ask()
    await userEvent.click(screen.getByTestId('browser-associate-deny'))

    expect(calls('browser_respond')).toEqual([{ id: 'ask-1', name: null }])
    expect(screen.queryByTestId('browser-associate-modal')).not.toBeInTheDocument()
  })

  it('replaces an ask still on screen with a newer one, and answers for that one', async () => {
    await ask()
    act(() => emitEvent(EVENTS.browserAssociate, { id: 'ask-2', key: 'ZZZZyyyyXXXXwwww0000=' }))

    expect(screen.getByTestId('browser-associate-fingerprint')).toHaveTextContent('ZZZZyyyy…000=')
    await userEvent.click(screen.getByTestId('browser-associate-allow'))
    expect(calls('browser_respond')).toEqual([{ id: 'ask-2', name: 'Browser' }])

    // The same extension asking again — a retry after a timeout — is a new
    // ask with an id of its own, and the answer names that one.
    act(() => emitEvent(EVENTS.browserAssociate, { id: 'ask-3', key: 'ZZZZyyyyXXXXwwww0000=' }))
    await userEvent.click(screen.getByTestId('browser-associate-allow'))
    expect(calls('browser_respond').slice(-1)).toEqual([{ id: 'ask-3', name: 'Browser' }])
  })

  it('leaves on its own once Rust has given up on the ask', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    try {
      await ask()
      act(() => {
        vi.advanceTimersByTime(ASSOCIATE_TIMEOUT_MS)
      })
      expect(screen.queryByTestId('browser-associate-modal')).not.toBeInTheDocument()
      expect(calls('browser_respond')).toEqual([])
    } finally {
      vi.useRealTimers()
    }
  })
})
