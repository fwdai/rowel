import { describe, it, expect, vi } from 'vitest'
import { act, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import App from '@/App'
import { enterMain, useApp, usePrefs, useUi } from '@/store'
import { unlistens } from './events'
import { appStatusDefault, calls, mockCommand } from './ipc'
import { seedApp } from './utils'

// Each `unlisten` the fake event bus handed out is a spy (see `test/events`).
const isCalled = (fn: () => void) => vi.mocked(fn).mock.calls.length > 0

// How the shell decides where to open: off the boot probe's answer, already in
// the store — or, when that never came, off one more probe of its own.
describe('App', () => {
  it('opens on the lock screen when the probe says there is a vault', () => {
    seedApp({ initialized: true })
    render(<App />)
    expect(useApp.getState().flow).toBe('auth')
    expect(calls('app_status')).toEqual([])
  })

  it('opens on setup when the probe says there is nothing on disk', () => {
    seedApp({ initialized: false })
    render(<App />)
    expect(useApp.getState().flow).toBe('setup')
  })

  it('asks again when the boot probe never answered, so a pristine install still reaches setup', async () => {
    useApp.setState({ status: null })
    mockCommand('app_status', () => ({ ...appStatusDefault(), initialized: false }))
    render(<App />)

    await waitFor(() => expect(useApp.getState().flow).toBe('setup'))
    expect(useApp.getState().status?.initialized).toBe(false)
  })

  // That late answer is also the first word on the stored preferences when
  // the boot probe failed: the theme and accent it carries have to reach the
  // document, not just the status slot.
  it('restores the saved theme and accent when its own probe is the one that answers', async () => {
    useApp.setState({ status: null })
    const status = appStatusDefault()
    mockCommand('app_status', () => ({
      ...status,
      settings: { ...status.settings, theme: 'dark', accent: 'ruby' }
    }))
    render(<App />)

    await waitFor(() => expect(usePrefs.getState().accent).toBe('ruby'))
    expect(usePrefs.getState().theme).toBe('dark')
    expect(document.documentElement.getAttribute('data-accent')).toBe('ruby')
    expect(document.documentElement.getAttribute('data-theme')).toBe('dark')
  })

  // `subscribeToEvents` hands back a cleanup that has to reach every one of the
  // subscriptions it started — each of which is a promise that may land after
  // the unmount. A shell that dropped one would, on the second mount, react to
  // every event twice.
  it('releases every event listener it took when it unmounts', async () => {
    const { unmount } = render(<App />)
    await waitFor(() => expect(unlistens).not.toHaveLength(0))
    const first = unlistens.length
    unmount()

    await waitFor(() => expect(unlistens.slice(0, first).every(fn => isCalled(fn))).toBe(true))

    render(<App />).unmount()
    await waitFor(() => expect(unlistens.every(fn => isCalled(fn))).toBe(true))
    // Both mounts subscribed, and neither left anything listening.
    expect(unlistens).toHaveLength(first * 2)
  })

  // A launch by double-clicking a backup: Rust parked the path because no
  // listener existed yet, and the shell collects it once it has subscribed.
  // Locked, it waits in the store for the unlock that can use it.
  it('collects the backup the app was launched with', async () => {
    seedApp({ initialized: true })
    mockCommand('take_opened_file', () => '/tmp/Rowel backup.rowel')
    render(<App />)

    await waitFor(() => expect(useApp.getState().openedFile).toBe('/tmp/Rowel backup.rowel'))
    expect(useApp.getState().flow).toBe('auth')
  })

  it('stays on the lock screen when the probe keeps failing', async () => {
    useApp.setState({ status: null })
    mockCommand('app_status', () => Promise.reject({ kind: 'other', message: 'no backend' }))
    render(<App />)

    await waitFor(() => expect(calls('app_status')).toHaveLength(1))
    expect(useApp.getState().flow).toBe('auth')
    expect(useApp.getState().status).toBeNull()
  })
})

describe('unlocking', () => {
  it('puts the caret in the search field', async () => {
    seedApp({ initialized: true })
    mockCommand('unlock', () => ({ entries: [], syncConfigured: false }))
    render(<App />)

    await userEvent.type(screen.getByPlaceholderText('Master Password'), 'hunter2{Enter}')

    await waitFor(() => expect(screen.getByTestId('search-input')).toHaveFocus(), {
      timeout: 3000
    })
  })

  it('leaves the search field alone when a waiting backup opens Settings', async () => {
    useApp.setState({ openedFile: '/tmp/Rowel backup.rowel' })

    await act(() => enterMain({ entries: [], syncConfigured: false, session: 1 }))

    expect(useUi.getState().settingsSection).toBe('import')
    expect(useUi.getState().searchFocus).toBe(0)
  })

  it('leaves the search field alone on mobile', async () => {
    vi.resetModules()
    vi.doMock('@/lib/platform', () => ({ isIOS: true, isAndroid: false, isMobile: true }))
    try {
      const store = await import('@/store')
      await act(() => store.enterMain({ entries: [], syncConfigured: false, session: 1 }))

      expect(store.useApp.getState().flow).toBe('main')
      expect(store.useUi.getState().searchFocus).toBe(0)
    } finally {
      vi.doUnmock('@/lib/platform')
      vi.resetModules()
    }
  })
})
