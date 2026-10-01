import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import type { Entry, EntryMeta, Passkey } from '@/api/types'
import { calls, mockCommand } from '../test/ipc'
import { toEntryMeta } from '../test/meta'
import { subscribeToEvents } from './events'
import {
  useApp,
  useUi,
  useVault,
  selectCurrent,
  setEntries,
  setArchive,
  setCurrentEntry,
  editEntry,
  saveEntry,
  deleteEntry,
  attachFile,
  removeAttachment,
  enterMain,
  setSyncStatus,
  initialApp,
  setFilterType,
  lockVault
} from './index'

const meta = (id: string, title = id): EntryMeta =>
  ({ id, type: 'login', title, tags: [], urlHost: '', favorite: false })

const current = () => selectCurrent(useVault.getState())

const connected = () => setSyncStatus({ ...initialApp.sync, configured: true })

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true })
  // A lock is an event: nothing reacts to one without the subscription.
  subscribeToEvents()
  // Echo back the saved entry's metadata, as the real backend does.
  mockCommand('save_entry', ({ entry }) => toEntryMeta(entry as Entry))
})

afterEach(() => vi.useRealTimers())

describe('saveEntry', () => {
  it('creates a new entry and selects it', async () => {
    await saveEntry({ type: 'login', title: 'New', username: 'u', password: 'p' })

    const { items } = useVault.getState()
    expect(items).toHaveLength(1)
    expect(items[0].title).toBe('New')
    expect(current()?.id).toBe(items[0].id)
    expect(calls('save_entry')).toHaveLength(1)
    // Nothing to sync to: this vault is local-only.
    await vi.advanceTimersByTimeAsync(60_000)
    expect(calls('sync_now')).toHaveLength(0)
  })

  it('updates an existing entry', async () => {
    setEntries([meta('a', 'Old')])
    await saveEntry({ id: 'a', type: 'login', title: 'Updated', username: 'u', password: 'p' })

    const { items } = useVault.getState()
    expect(items).toHaveLength(1)
    expect(items[0].title).toBe('Updated')
  })

  it('drops a kind filter that would hide the entry just saved', async () => {
    setFilterType('login')

    await saveEntry({
      type: 'card',
      title: 'Travel Card',
      number: '4111111111111111',
      month: '12',
      year: '30',
      cvc: '123'
    })

    // The row has to be visible for the selection to mean anything.
    expect(useUi.getState().filterType).toBeNull()
    expect(current()?.title).toBe('Travel Card')
  })

  it('keeps a kind filter the saved entry still matches', async () => {
    setFilterType('login')

    await saveEntry({ type: 'login', title: 'New', username: 'u', password: 'p' })

    expect(useUi.getState().filterType).toBe('login')
    expect(current()?.title).toBe('New')
  })

  // A draft spread from a revealed login carries its passkeys, and they reach
  // the backend untouched — while staying out of the list metadata.
  it('carries a login draft passkeys through to the backend', async () => {
    const passkeys: Passkey[] = [
      {
        credentialId: 'Y3JlZDE',
        rpId: 'acme.test',
        userHandle: 'dWgx',
        userName: 'alice',
        userDisplayName: 'Alice',
        counter: 0
      }
    ]

    await saveEntry({ type: 'login', title: 'Acme', username: 'u', password: 'p', passkeys })

    const saved = calls('save_entry')[0].entry as Entry
    expect(saved.type === 'login' && saved.passkeys).toEqual(passkeys)
    expect(useVault.getState().items[0]).not.toHaveProperty('passkeys')
  })
})

describe('selection', () => {
  it('resolves the selected row across live rows and tombstones', () => {
    setEntries([meta('a')])
    setArchive([{ ...meta('t'), deletedAt: '2024-01-05T00:00:00.000Z' }])

    setCurrentEntry('t')
    expect(current()?.id).toBe('t')
  })

  it('refuses to open a tombstone in the editor', () => {
    setArchive([{ ...meta('t'), deletedAt: '2024-01-05T00:00:00.000Z' }])
    setCurrentEntry('t')

    editEntry()
    expect(useVault.getState().editing).toBe(false)
  })

  it('ends an edit whose row a list replacement dropped', () => {
    setEntries([meta('a')])
    setCurrentEntry('a')
    editEntry()
    expect(useVault.getState().editing).toBe(true)

    setEntries([])

    expect(useVault.getState().editing).toBe(false)
    expect(current()).toBeNull()
  })
})

describe('auto-sync', () => {
  it('debounces a burst of writes into a single push', async () => {
    connected()

    await saveEntry({ type: 'login', title: 'One', username: 'u', password: 'p' })
    await vi.advanceTimersByTimeAsync(1_500)
    await saveEntry({ type: 'login', title: 'Two', username: 'u', password: 'p' })

    // The second write reset the timer, so nothing has gone out yet.
    await vi.advanceTimersByTimeAsync(1_500)
    expect(calls('sync_now')).toHaveLength(0)

    await vi.advanceTimersByTimeAsync(1_500)
    expect(calls('sync_now')).toHaveLength(1)
  })

  it('publishes a save within a couple of seconds', async () => {
    connected()

    await saveEntry({ type: 'login', title: 'One', username: 'u', password: 'p' })
    await vi.advanceTimersByTimeAsync(2_000)
    expect(calls('sync_now')).toHaveLength(1)
  })

  it('drops a write still waiting when the vault locks', async () => {
    connected()

    await saveEntry({ type: 'login', title: 'One', username: 'u', password: 'p' })
    await lockVault()
    // The waiting write is dropped by the reaction to `vault:locked`, which
    // lands after the command resolves — as it does in the real app.
    await vi.waitFor(() => expect(useApp.getState().flow).toBe('auth'))

    // The key is gone: a push fired now could only fail, and the next unlock
    // syncs anyway.
    await vi.advanceTimersByTimeAsync(60_000)
    expect(calls('sync_now')).toHaveLength(0)
  })

  it('publishes a delete too', async () => {
    connected()
    setEntries([meta('a')])

    await deleteEntry('a')
    await vi.advanceTimersByTimeAsync(2_000)
    expect(calls('sync_now')).toHaveLength(1)
  })

  // A file is a write to the vault like any other, even with no entry changed.
  it('publishes an attachment added or removed', async () => {
    connected()

    await attachFile('a', '/Users/me/scan.pdf')
    await vi.advanceTimersByTimeAsync(2_000)
    expect(calls('sync_now')).toHaveLength(1)

    await removeAttachment('a1')
    await vi.advanceTimersByTimeAsync(2_000)
    expect(calls('sync_now')).toHaveLength(2)
  })
})

describe('deleteEntry', () => {
  it('removes the entry and clears the selection', async () => {
    setEntries([meta('a'), meta('b')])
    await deleteEntry('a')

    expect(useVault.getState().items.map(e => e.id)).toEqual(['b'])
    expect(current()).toBeNull()
  })
})

describe('enterMain', () => {
  it('loads the vault and switches to the main flow', async () => {
    await enterMain({ entries: [meta('a')], syncConfigured: true })

    expect(useApp.getState().flow).toBe('main')
    expect(useVault.getState().items.map(e => e.id)).toEqual(['a'])
    // A configured vault syncs once on unlock, before any local write.
    expect(useApp.getState().sync.configured).toBe(true)
    expect(calls('sync_now')).toHaveLength(1)
  })

  it('leaves sync off for a vault that has never been connected', async () => {
    await enterMain({ entries: [], syncConfigured: false })

    expect(useApp.getState().sync.configured).toBe(false)
    expect(calls('sync_now')).toHaveLength(0)
  })
})
