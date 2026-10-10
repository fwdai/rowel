import { describe, it, expect, beforeEach, vi } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Main from '@/components/Main'
import Show from '@/components/Main/Body/Aside/Show'
import type { Entry, PasswordHistoryItem } from '@/api/types'
import { setCurrentEntry } from '@/store'
import { withEntries, loginEntry, loginMeta } from './utils'
import { setLayout } from './layout'
import { calls, mockCommand } from './ipc'
import { toEntryMeta } from './meta'

const HOUR = 60 * 60 * 1000
const ago = (ms: number) => new Date(Date.now() - ms).toISOString()

const HISTORY: PasswordHistoryItem[] = [
  { password: 'second-old', replacedAt: ago(3 * HOUR) },
  { password: 'first-old', replacedAt: ago(48 * HOUR) }
]

const withHistory = (overrides: Partial<Entry> = {}) =>
  loginEntry({ password_updated_at: ago(3 * HOUR), passwordHistory: HISTORY, ...overrides })

beforeEach(() => {
  vi.clearAllMocks()
  setLayout('wide')
})

describe('Password history', () => {
  it('shows nothing new when there is no history', async () => {
    mockCommand('reveal_entry', () => loginEntry({ password_updated_at: ago(3 * HOUR) }))
    render(<Show entry={loginMeta()} />)

    expect(await screen.findByText('Changed 3h ago')).toBeInTheDocument()
    expect(screen.queryByTestId('password-history-toggle')).not.toBeInTheDocument()
  })

  // Closed by default: the stamp gains a count and nothing else is on screen.
  it('folds the history behind the rotation stamp', async () => {
    mockCommand('reveal_entry', () => withHistory())
    render(<Show entry={loginMeta()} />)

    const toggle = await screen.findByTestId('password-history-toggle')
    expect(toggle).toHaveTextContent('Changed 3h ago · 2 previous')
    expect(toggle).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByTestId('password-history')).not.toBeInTheDocument()

    await userEvent.click(toggle)
    expect(toggle).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByTestId('password-history')).toBeInTheDocument()

    await userEvent.click(toggle)
    expect(screen.queryByTestId('password-history')).not.toBeInTheDocument()
  })

  it('still offers the history when the login has no rotation stamp', async () => {
    mockCommand('reveal_entry', () => loginEntry({ passwordHistory: HISTORY }))
    render(<Show entry={loginMeta()} />)

    expect(await screen.findByTestId('password-history-toggle')).toHaveTextContent(
      /^2 previous$/
    )
  })

  // A cleared password still has the ones before it: the row stays, with
  // nothing to show, reveal or copy, only the way in to the history.
  it('keeps the history reachable when the password is cleared', async () => {
    let entry = withHistory({ password: '' })
    mockCommand('reveal_entry', () => entry)
    mockCommand('clear_password_history', () => {
      entry = loginEntry({ password: '', updatedAt: new Date().toISOString() })
      return toEntryMeta(entry)
    })
    withEntries([loginMeta()])
    setCurrentEntry('l1')
    render(<Main />)

    const toggle = await screen.findByTestId('password-history-toggle')
    expect(toggle).toHaveTextContent('Changed 3h ago · 2 previous')
    expect(screen.queryByTestId('entry-value-password')).not.toBeInTheDocument()
    expect(screen.queryByTestId('reveal-password')).not.toBeInTheDocument()
    // With no value, the toggle is the row's control: its label names it.
    expect(screen.getByLabelText('Password')).toBe(toggle)

    await userEvent.click(toggle)
    await userEvent.click(screen.getByTestId('password-history-reveal-0'))
    expect(screen.getByTestId('password-history-value-0')).toHaveTextContent('second-old')
    await userEvent.click(screen.getByTestId('password-history-copy-1'))
    expect(calls('copy_to_clipboard')).toContainEqual(
      expect.objectContaining({ value: 'first-old' })
    )

    await userEvent.click(screen.getByTestId('password-history-clear'))
    await userEvent.click(screen.getByTestId('password-history-clear'))
    expect(calls('clear_password_history')).toEqual([{ id: 'l1' }])
    // Nothing kept and nothing current: the row is gone again.
    await waitFor(() =>
      expect(screen.queryByTestId('password-history-toggle')).not.toBeInTheDocument()
    )
    expect(screen.queryByText('Password')).not.toBeInTheDocument()
  })

  // Masked by default, each on its own; revealed one at a time, copied either way.
  it('lists previous passwords masked, with reveal and copy', async () => {
    mockCommand('reveal_entry', () => withHistory())
    render(<Show entry={loginMeta()} />)
    await userEvent.click(await screen.findByTestId('password-history-toggle'))

    const first = screen.getByTestId('password-history-value-0')
    expect(first).not.toHaveTextContent('second-old')
    expect(screen.getByText('Replaced 3 hours ago')).toBeInTheDocument()

    await userEvent.click(screen.getByTestId('password-history-reveal-0'))
    expect(first).toHaveTextContent('second-old')
    expect(screen.getByTestId('password-history-value-1')).not.toHaveTextContent('first-old')

    await userEvent.click(screen.getByTestId('password-history-copy-1'))
    expect(calls('copy_to_clipboard')).toContainEqual(
      expect.objectContaining({ value: 'first-old' })
    )

    // On the phone the value is the row's only copy control: a press on it
    // copies, and the copy button sits in the phone-hidden wrapper
    // (`ROW_COPY`; jsdom applies no stylesheet, so the wrapper is the
    // assertion), like every other row's.
    await userEvent.click(first)
    expect(calls('copy_to_clipboard')).toContainEqual(
      expect.objectContaining({ value: 'second-old' })
    )
    expect(screen.getByTestId('password-history-copy-0').closest('.max-md\\:hidden')).not.toBeNull()
  })

  // Two presses, like the editor's Cancel: the first only asks.
  it('clears the history after a confirmation, and the reveal comes back without it', async () => {
    let entry = withHistory()
    mockCommand('reveal_entry', () => entry)
    mockCommand('clear_password_history', () => {
      entry = loginEntry({
        password_updated_at: ago(3 * HOUR),
        updatedAt: new Date().toISOString()
      })
      return toEntryMeta(entry)
    })
    withEntries([loginMeta()])
    setCurrentEntry('l1')
    render(<Main />)

    await userEvent.click(await screen.findByTestId('password-history-toggle'))
    const clear = screen.getByTestId('password-history-clear')
    await userEvent.click(clear)
    expect(calls('clear_password_history')).toHaveLength(0)
    expect(clear).toHaveTextContent('Clear all previous passwords?')

    await userEvent.click(clear)
    expect(calls('clear_password_history')).toEqual([{ id: 'l1' }])
    // The row's `updatedAt` moved, so the entry revealed itself again.
    expect(await screen.findByText('Changed 3h ago')).toBeInTheDocument()
    expect(screen.queryByTestId('password-history-toggle')).not.toBeInTheDocument()
  })

  it('says so when the clear fails', async () => {
    mockCommand('reveal_entry', () => withHistory())
    mockCommand('clear_password_history', () =>
      Promise.reject({ kind: 'other', message: 'refused' })
    )
    render(<Show entry={loginMeta()} />)

    await userEvent.click(await screen.findByTestId('password-history-toggle'))
    await userEvent.click(screen.getByTestId('password-history-clear'))
    await userEvent.click(screen.getByTestId('password-history-clear'))
    expect(
      await screen.findByText('Could not clear the history. Please try again.')
    ).toBeInTheDocument()
  })

  // The editor is about the password being set; the history stays on the read face.
  it('is not part of the editor', async () => {
    mockCommand('reveal_entry', () => withHistory())
    mockCommand('save_entry', ({ entry }) => toEntryMeta(entry as Entry))
    render(<Show entry={loginMeta()} editing />)

    expect(await screen.findByText('Changed 3h ago')).toBeInTheDocument()
    expect(screen.queryByTestId('password-history-toggle')).not.toBeInTheDocument()
  })

  // The phone draws the same disclosure inline in its read screen.
  it('opens inline on the phone too', async () => {
    setLayout('compact')
    mockCommand('reveal_entry', () => withHistory())
    withEntries([loginMeta()])
    setCurrentEntry('l1')
    render(<Main />)

    await userEvent.click(await screen.findByTestId('password-history-toggle'))
    expect(screen.getByTestId('password-history')).toBeInTheDocument()
    expect(screen.getByTestId('password-history-clear')).toBeInTheDocument()
  })

  it('keeps the history reachable on the phone when the password is cleared', async () => {
    setLayout('compact')
    mockCommand('reveal_entry', () => withHistory({ password: '' }))
    withEntries([loginMeta()])
    setCurrentEntry('l1')
    render(<Main />)

    await userEvent.click(await screen.findByTestId('password-history-toggle'))
    expect(screen.getByTestId('password-history')).toBeInTheDocument()
  })
})
