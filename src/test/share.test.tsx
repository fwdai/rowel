import { describe, it, expect, beforeEach, vi } from 'vitest'
import { act } from 'react'
import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import Main from '@/components/Main'
import Settings from '@/components/Main/Sidebar/Settings'
import {
  useUi,
  useVault,
  selectCurrent,
  openSend,
  openReceive,
  openSettings,
  closeSend,
  initialApp,
  setSyncStatus,
  DEFAULT_PREFS
} from '@/store'
import type { Entry } from '@/api/types'
import type { ActiveShare, ShareCreated } from '@/api/share'
import { withEntries, loginMeta, deferred } from './utils'
import { calls, mockCommandOnce } from './ipc'

const LINK = 'rowel://share#v1.file-1.a2V5LTFrZXktMWtleS0xa2V5LTFrZXktMWtleS0xa2V5'

const created = (fileId: string): ShareCreated => ({
  link: `rowel://share#v1.${fileId}.a2V5`,
  fileId,
  expiresAt: '2024-01-02T00:00:00.000Z'
})

const seed = ({ connected = true } = {}) => {
  withEntries([loginMeta({ id: 'l1', title: 'Google' })])
  if (connected) setSyncStatus({ ...initialApp.sync, configured: true })
}

// A share still N hours out. Half an hour past the mark so the floor in
// `relativeUntil` cannot land a millisecond short and read one hour less.
const inHours = (hours: number) =>
  new Date(Date.now() + (hours + 0.5) * 3_600_000).toISOString()

beforeEach(() => vi.clearAllMocks())

describe('sharing an entry', () => {
  it('asks for Drive rather than offering a link it cannot make', async () => {
    seed({ connected: false })
    render(<Main />)

    openSend('l1')

    expect(await screen.findByTestId('share-needs-drive')).toHaveTextContent(
      'Connect Google Drive to share'
    )
    // Nothing is sealed, and nothing is uploaded, until there is a Drive to put
    // it in — the dialog explains instead of failing at the backend.
    expect(calls('share_create')).toHaveLength(0)
  })

  it('seals on open and shows the link', async () => {
    seed()
    render(<Main />)

    openSend('l1')

    const field = await screen.findByTestId('share-link')
    expect(calls('share_create')).toContainEqual({ entryId: 'l1' })
    expect(field).toHaveValue(LINK)
    expect(field).toHaveAttribute('readonly')
    expect(screen.getByTestId('share-send-modal')).toHaveAccessibleName('Share this entry')
    expect(
      screen.getByText('Rowel stops opening this link in 24 hours. Revoke it to delete the file now.')
    ).toBeInTheDocument()
  })

  it('copies the link under the clipboard timeout the user chose', async () => {
    seed()
    render(<Main />)
    openSend('l1')
    await screen.findByTestId('share-link')

    await userEvent.click(screen.getByTestId('share-copy-button'))

    // The link opens the entry for anyone holding it, so it leaves the
    // clipboard on the same timer as the secrets it stands in for.
    expect(calls('copy_to_clipboard')).toContainEqual(
      { value: LINK, clearAfterMs: DEFAULT_PREFS.clipboardTimeoutMs }
    )
    expect(screen.getByTestId('share-copy-button')).toHaveTextContent('Copied')
  })

  it('shows the link of the entry being shared, and takes back the one nobody saw', async () => {
    const first = deferred<ShareCreated>()
    const second = deferred<ShareCreated>()
    mockCommandOnce('share_create', () => first.promise)
    mockCommandOnce('share_create', () => second.promise)
    seed()
    render(<Main />)

    openSend('l1')
    await screen.findByTestId('share-send-loading')
    act(() => openSend('l2'))
    await act(async () => second.resolve(created('file-b')))
    // The first seal lands last, on a dialog that has moved to another entry.
    await act(async () => first.resolve(created('file-a')))

    expect(screen.getByTestId('share-link')).toHaveValue(created('file-b').link)
    // Its link was never on screen and never will be, so the file it points at
    // must not be left sitting in the sender's Drive for 24 hours.
    await vi.waitFor(() => expect(calls('share_revoke')).toContainEqual({ fileId: 'file-a' }))
    expect(calls('share_revoke')).toHaveLength(1)
    await vi.waitFor(() => expect(useUi.getState().orphans).toEqual([]))
  })

  it('remembers a share it could not take back, and tries again on the next dialog', async () => {
    const first = deferred<ShareCreated>()
    mockCommandOnce('share_create', () => first.promise)
    mockCommandOnce('share_create', () => created('file-b'))
    mockCommandOnce('share_revoke', () => Promise.reject({ kind: 'io', message: 'offline' }))
    seed()
    render(<Main />)

    openSend('l1')
    await screen.findByTestId('share-send-loading')
    act(() => openSend('l2'))
    await screen.findByTestId('share-link')
    await act(async () => first.resolve(created('file-a')))

    // The take-back failed, so the share is still live in Drive and this is the
    // only record that it exists.
    await vi.waitFor(() => expect(calls('share_revoke')).toContainEqual({ fileId: 'file-a' }))
    expect(useUi.getState().orphans).toEqual(['file-a'])

    // Opening any share dialog is the next chance to make it right.
    act(() => closeSend())
    act(() => openSend('l1'))

    await vi.waitFor(() => expect(calls('share_revoke')).toHaveLength(2))
    expect(calls('share_revoke')[0]).toEqual({ fileId: 'file-a' })
    await vi.waitFor(() => expect(useUi.getState().orphans).toEqual([]))
  })

  it('revokes the link and closes', async () => {
    seed()
    render(<Main />)
    openSend('l1')
    await screen.findByTestId('share-link')

    await userEvent.click(screen.getByTestId('share-revoke-button'))

    expect(calls('share_revoke')).toContainEqual({ fileId: 'file-1' })
    expect(screen.queryByTestId('share-send-modal')).not.toBeInTheDocument()
    expect(useUi.getState().sendFor).toBeNull()
  })

  it('says in the app’s own words why the seal failed', async () => {
    mockCommandOnce('share_create', () =>
      Promise.reject({ kind: 'syncNotConfigured', message: 'sync is not configured' })
    )
    seed()
    render(<Main />)

    openSend('l1')

    expect(await screen.findByTestId('share-send-error')).toHaveTextContent('Sync is not set up')
    await userEvent.click(screen.getByTestId('share-retry-button'))
    expect(await screen.findByTestId('share-link')).toHaveValue(LINK)
  })

  it('says why a passkey-only login cannot be shared, and offers the way out', async () => {
    const refusal = 'a login whose only secret is a passkey cannot be shared'
    mockCommandOnce('share_create', () =>
      Promise.reject({ kind: 'unsupported', message: refusal })
    )
    seed()
    render(<Main />)

    openSend('l1')

    expect(await screen.findByTestId('share-send-error')).toHaveTextContent(refusal)
    expect(screen.getByTestId('share-close-button')).toBeInTheDocument()
    expect(screen.queryByTestId('share-link')).not.toBeInTheDocument()
  })

  it('retries the revoke that failed rather than sealing a second link', async () => {
    mockCommandOnce('share_revoke', () =>
      Promise.reject({ kind: 'io', message: 'drive is unreachable' })
    )
    seed()
    render(<Main />)
    openSend('l1')
    await screen.findByTestId('share-link')

    await userEvent.click(screen.getByTestId('share-revoke-button'))
    expect(await screen.findByTestId('share-send-error')).toHaveTextContent('drive is unreachable')
    // The link is still live, so it is still on screen — with the way to take
    // it back still under it.
    expect(screen.getByTestId('share-link')).toHaveValue(LINK)

    await userEvent.click(screen.getByTestId('share-revoke-button'))

    expect(calls('share_revoke')).toHaveLength(2)
    // A second seal would have published a second link while the first was
    // still out there.
    expect(calls('share_create')).toHaveLength(1)
    expect(screen.queryByTestId('share-send-modal')).not.toBeInTheDocument()
  })

  it('is offered from the detail header of a live entry', async () => {
    seed()
    render(<Main />)
    await userEvent.click(await screen.findByTestId('entry-item'))

    await userEvent.click(await screen.findByTestId('more-actions-button'))
    await userEvent.click(screen.getByTestId('share-entry-button'))

    expect(useUi.getState().sendFor).toBe('l1')
  })
})

describe('receiving a shared entry', () => {
  const paste = async (link = LINK) => {
    openReceive()
    await userEvent.type(await screen.findByTestId('share-link-input'), link)
    await userEvent.click(screen.getByTestId('share-open-button'))
  }

  it('is reachable from the kind picker', async () => {
    seed()
    render(<Main />)
    await userEvent.click(screen.getByTestId('add-entry-button'))

    await userEvent.click(screen.getByTestId('add-receive-share'))

    expect(useUi.getState().receiveOpen).toBe(true)
    expect(useUi.getState().addPicker).toBe(false)
  })

  it('says in the app’s own words that the share is gone', async () => {
    mockCommandOnce('share_open', () =>
      Promise.reject({ kind: 'shareExpired', message: 'this share has expired or was revoked' })
    )
    seed()
    render(<Main />)

    await paste('not-a-link')

    expect(await screen.findByTestId('share-receive-error')).toHaveTextContent(
      'This share has expired or was revoked'
    )
    expect(screen.queryByTestId('share-preview')).not.toBeInTheDocument()
  })

  it('previews without showing a single secret', async () => {
    seed()
    render(<Main />)

    await paste()

    const preview = within(await screen.findByTestId('share-preview'))
    expect(preview.getByTestId('share-preview-title')).toHaveTextContent('Shared Netflix')
    expect(preview.getByText('shared@example.com')).toBeInTheDocument()
    expect(preview.getByText('https://netflix.com')).toBeInTheDocument()
    // The password is counted, never drawn.
    expect(preview.getByTestId('share-preview-secrets')).toHaveTextContent(
      '1 secret field included'
    )
    expect(screen.queryByText('from-a-friend')).not.toBeInTheDocument()
  })

  it('adds it as a new entry of its own', async () => {
    seed()
    render(<Main />)
    await paste()
    await screen.findByTestId('share-preview')

    await userEvent.click(screen.getByTestId('share-add-button'))

    expect(calls('save_entry')).toHaveLength(1)
    const saved = calls('save_entry')[0].entry as unknown as Record<string, unknown>
    // The sender's row id never travels; this vault mints its own.
    expect(saved.id).toEqual(expect.any(String))
    expect(saved.id).not.toBe('')
    expect(saved.createdAt).toEqual(expect.any(String))
    expect(saved).toMatchObject({
      type: 'login',
      title: 'Shared Netflix',
      username: 'shared@example.com',
      password: 'from-a-friend'
    })

    await vi.waitFor(() => {
      expect(useUi.getState().receiveOpen).toBe(false)
      expect(selectCurrent(useVault.getState())?.title).toBe('Shared Netflix')
    })
  })

  it('keeps nothing of the last link once the dialog is closed', async () => {
    seed()
    render(<Main />)
    await paste()
    await screen.findByTestId('share-preview')

    await userEvent.click(screen.getByTestId('share-cancel-button'))
    act(() => openReceive())

    // Someone else's secret does not sit in memory behind a closed dialog, and
    // reopening asks the one question this dialog is for.
    const input = await screen.findByTestId('share-link-input')
    expect(input).toHaveValue('')
    expect(screen.queryByTestId('share-preview')).not.toBeInTheDocument()
    expect(screen.getByTestId('share-open-button')).toBeInTheDocument()
  })

  it('takes the secret and none of the sender claims about it', async () => {
    mockCommandOnce('share_open', () => ({
      id: 'existing-id',
      type: 'login',
      title: 'Shared Netflix',
      website: 'https://netflix.com',
      username: 'shared@example.com',
      password: 'from-a-friend',
      email: '',
      note: '',
      otp: '',
      createdAt: '2019-01-01T00:00:00.000Z',
      updatedAt: '2019-01-01T00:00:00.000Z',
      password_updated_at: '2019-01-01T00:00:00.000Z',
      favorite: true,
      passkeys: [{ id: 'pk1', rpId: 'netflix.com' }]
    } as unknown as Entry))
    seed()
    render(<Main />)
    await paste()
    await screen.findByTestId('share-preview')

    await userEvent.click(screen.getByTestId('share-add-button'))

    const saved = calls('save_entry')[0].entry as unknown as Record<string, unknown>
    expect(saved.password).toBe('from-a-friend')
    // An id chosen by the sender is an id that could name a row already here.
    expect(saved.id).toEqual(expect.any(String))
    expect(saved.id).not.toBe('existing-id')
    expect(saved.id).not.toBe('')
    expect(saved.favorite).toBeFalsy()
    expect(saved).not.toHaveProperty('passkeys')
    expect(saved).not.toHaveProperty('password_updated_at')
    expect(saved.createdAt).not.toBe('2019-01-01T00:00:00.000Z')
  })
})

describe('Settings › Shared links', () => {
  const expand = async () => {
    seed()
    render(<Settings />)
    openSettings('sync')
    // The row's button is the disclosure.
    const row = await screen.findByTestId('settings-shares-row-toggle')
    expect(row).toHaveAttribute('aria-expanded', 'false')
    await userEvent.click(row)
    expect(row).toHaveAttribute('aria-expanded', 'true')
  }

  it('is not offered without Drive', async () => {
    seed({ connected: false })
    render(<Settings />)
    openSettings('sync')

    expect(await screen.findByTestId('settings-drive-row')).toBeInTheDocument()
    expect(screen.queryByTestId('settings-shares-row')).not.toBeInTheDocument()
  })

  it('lists live links, naming the entry each came from', async () => {
    mockCommandOnce('share_list', () => ([
      {
        fileId: 'f1',
        entryId: 'l1',
        kind: 'login',
        createdAt: '2024-01-01T00:00:00.000Z',
        expiresAt: inHours(3)
      },
      {
        fileId: 'f2',
        entryId: 'gone',
        kind: 'card',
        createdAt: '2024-01-01T00:00:00.000Z',
        expiresAt: inHours(20)
      }
    ]))

    await expand()

    const first = within(await screen.findByTestId('settings-share-f1'))
    expect(first.getByText('Google')).toBeInTheDocument()
    expect(screen.getByTestId('settings-share-f1')).toHaveTextContent(
      'Login · Expires in 3 hours'
    )
    // A share outlives the entry it was cut from, and says so.
    expect(within(screen.getByTestId('settings-share-f2')).getByText('Deleted entry'))
      .toBeInTheDocument()
  })

  it('revokes one and drops it from the list', async () => {
    mockCommandOnce('share_list', () => ([
      {
        fileId: 'f1',
        entryId: 'l1',
        kind: 'login',
        createdAt: '2024-01-01T00:00:00.000Z',
        expiresAt: inHours(3)
      }
    ]))

    await expand()
    await screen.findByTestId('settings-share-f1')

    await userEvent.click(screen.getByTestId('settings-share-revoke-f1'))

    expect(calls('share_revoke')).toContainEqual({ fileId: 'f1' })
    expect(await screen.findByTestId('settings-shares-empty')).toHaveTextContent(
      'No active links'
    )
  })

  it('says so when there is nothing out there', async () => {
    await expand()

    expect(await screen.findByTestId('settings-shares-empty')).toBeInTheDocument()
    expect(calls('share_list').length).toBeGreaterThan(0)
  })

  it('keeps the countdown honest while the row stays open', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    try {
      const share = (expiresAt: string) => ({
        fileId: 'f1',
        entryId: 'l1',
        kind: 'login' as const,
        createdAt: '2024-01-01T00:00:00.000Z',
        expiresAt
      })
      // Two minutes and a half out, then — once the row has watched it run
      // out — gone from the backend's own listing.
      mockCommandOnce('share_list', () => [share(new Date(Date.now() + 150_000).toISOString())])
      mockCommandOnce('share_list', () => [])
      const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime })
      seed()
      render(<Settings />)
      openSettings('sync')
      await user.click(await screen.findByTestId('settings-shares-row-toggle'))
      expect(await screen.findByTestId('settings-share-f1')).toHaveTextContent(
        'Expires in 2 minutes'
      )

      // Nothing else re-renders this row; only its own clock can.
      await act(async () => {
        vi.advanceTimersByTime(60_000)
      })
      expect(screen.getByTestId('settings-share-f1')).toHaveTextContent('Expires in 1 minute')

      await act(async () => {
        vi.advanceTimersByTime(120_000)
      })
      // Past the mark the row asks again instead of showing "Expired" for a
      // file the sweep has already removed.
      expect(await screen.findByTestId('settings-shares-empty')).toBeInTheDocument()
      expect(calls('share_list')).toHaveLength(2)
    } finally {
      vi.useRealTimers()
    }
  })

  it('shows a refresh that failed instead of a stale "Expired" row, and recovers', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    try {
      const share = {
        fileId: 'f1',
        entryId: 'l1',
        kind: 'login' as const,
        createdAt: '2024-01-01T00:00:00.000Z',
        expiresAt: new Date(Date.now() + 90_000).toISOString()
      }
      // Listed, then Drive is unreachable when the countdown runs out, then
      // back a minute later with the swept, empty list.
      mockCommandOnce('share_list', () => [share])
      mockCommandOnce('share_list', () =>
        Promise.reject({ kind: 'other', message: 'Drive API 503' })
      )
      mockCommandOnce('share_list', () => [])
      const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime })
      seed()
      render(<Settings />)
      openSettings('sync')
      await user.click(await screen.findByTestId('settings-shares-row-toggle'))
      expect(await screen.findByTestId('settings-share-f1')).toBeInTheDocument()

      await act(async () => {
        vi.advanceTimersByTime(120_000)
      })
      // The failure is what is shown — not a row claiming "Expired" about a
      // file nobody could confirm anything about.
      expect(await screen.findByTestId('settings-shares-error')).toHaveTextContent('Drive API 503')
      expect(screen.queryByTestId('settings-share-f1')).not.toBeInTheDocument()

      await act(async () => {
        vi.advanceTimersByTime(60_000)
      })
      expect(await screen.findByTestId('settings-shares-empty')).toBeInTheDocument()
      expect(screen.queryByTestId('settings-shares-error')).not.toBeInTheDocument()
      expect(calls('share_list')).toHaveLength(3)
    } finally {
      vi.useRealTimers()
    }
  })

  it('lets only the newest of overlapping refreshes answer', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    try {
      const share = {
        fileId: 'f1',
        entryId: 'l1',
        kind: 'login' as const,
        createdAt: '2024-01-01T00:00:00.000Z',
        expiresAt: new Date(Date.now() + 90_000).toISOString()
      }
      // Listed, then two refreshes in flight at once: the first tick's, which
      // hangs, and the next tick's, which comes back promptly and swept.
      const slow = deferred<ActiveShare[]>()
      const fast = deferred<ActiveShare[]>()
      mockCommandOnce('share_list', () => [share])
      mockCommandOnce('share_list', () => slow.promise)
      mockCommandOnce('share_list', () => fast.promise)
      const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime })
      seed()
      render(<Settings />)
      openSettings('sync')
      await user.click(await screen.findByTestId('settings-shares-row-toggle'))
      expect(await screen.findByTestId('settings-share-f1')).toBeInTheDocument()

      await act(async () => {
        vi.advanceTimersByTime(120_000)
      })
      await act(async () => {
        vi.advanceTimersByTime(60_000)
      })
      expect(calls('share_list')).toHaveLength(3)

      await act(async () => {
        fast.resolve([])
      })
      expect(await screen.findByTestId('settings-shares-empty')).toBeInTheDocument()

      // The older answer arrives last, and is nobody's answer: the share it
      // still lists was swept between the two requests.
      await act(async () => {
        slow.resolve([share])
      })
      expect(screen.getByTestId('settings-shares-empty')).toBeInTheDocument()
      expect(screen.queryByTestId('settings-share-f1')).not.toBeInTheDocument()
    } finally {
      vi.useRealTimers()
    }
  })
})
