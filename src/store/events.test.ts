import { describe, it, expect, beforeEach, vi } from 'vitest'
import { on, EVENTS, type EventName, type EventPayloads } from '@/api/events'
import type { EntryMeta } from '@/api/types'
import { appStatusDefault, calls, clearCalls, mockCommand } from '../test/ipc'
import { deferred } from '../test/utils'
import { subscribeToEvents } from './events'
import {
  useApp,
  useUi,
  useVault,
  selectCurrent,
  setEntries,
  setArchive,
  setView,
  setCurrentEntry,
  flowMain,
  initialApp,
  lockVault,
  setApp,
  enterMain,
  switchWorkspace
} from './index'

const meta = (id: string): EntryMeta => ({
  id,
  type: 'login',
  title: id,
  tags: [],
  urlHost: '',
  favorite: false
})

const current = () => selectCurrent(useVault.getState())

// The handler `subscribeToEvents` registered for one event. `on` is mocked
// globally (src/test/setup.ts), so the subscription is inspectable without a
// Tauri runtime.
const handlerFor = <E extends EventName>(event: E) => {
  const call = vi.mocked(on).mock.calls.find(([name]) => name === event)
  if (!call) throw new Error(`nothing subscribed to ${event}`)
  return call[1] as (payload: EventPayloads[E]) => void
}

// What `app_status` answers about the biometric gate.
const gate = (available: boolean) =>
  mockCommand('app_status', () => ({
    ...appStatusDefault(),
    biometric: { available, canEnroll: available, type: 'touch', mode: null }
  }))

beforeEach(() => {
  vi.clearAllMocks()
  subscribeToEvents()
})

describe('vault:merged', () => {
  it('adopts the entries a sync pulled in from another device', () => {
    setEntries([meta('a')])

    handlerFor(EVENTS.vaultMerged)({ entries: [meta('a'), meta('b')] })

    expect(useVault.getState().items.map(e => e.id)).toEqual(['a', 'b'])
  })

  // The selection is an id, so the row it names is whatever the merged list
  // says it is now.
  it('reads the selected row from the merged list', () => {
    setEntries([meta('a')])
    setCurrentEntry('a')

    handlerFor(EVENTS.vaultMerged)({ entries: [{ ...meta('a'), title: 'Renamed' }] })

    expect(current()?.title).toBe('Renamed')
  })

  it('clears the selection when the merge dropped that row', () => {
    setEntries([meta('a'), meta('b')])
    setCurrentEntry('b')

    handlerFor(EVENTS.vaultMerged)({ entries: [meta('a')] })

    expect(useVault.getState().currentId).toBeNull()
    expect(current()).toBeNull()
  })

  it('re-runs the audit, since the new rows have no strength result yet', () => {
    handlerFor(EVENTS.vaultMerged)({ entries: [meta('b')] })
    expect(calls('get_audit')).toHaveLength(1)
  })

  // A merge can add or drop tombstones too, and the Archive only loads on entry
  // — so an open one has to be told, while a closed one refetches on its own.
  it('re-reads the tombstones when the Archive is the open view', () => {
    setView('archive')
    clearCalls('list_deleted')

    handlerFor(EVENTS.vaultMerged)({ entries: [meta('b')] })

    expect(calls('list_deleted')).toHaveLength(1)
  })

  it('leaves the tombstones alone when the Archive is not open', () => {
    setView('items')
    clearCalls('list_deleted')

    handlerFor(EVENTS.vaultMerged)({ entries: [meta('b')] })

    expect(calls('list_deleted')).toHaveLength(0)
  })
})

// The backend owns sync — the consent flow, the runs, their outcome — and
// reports the whole of it on every change. The frontend stores it verbatim.
describe('sync:status', () => {
  it('is adopted as-is', () => {
    const status = {
      configured: true,
      provider: 'dropbox' as const,
      pending: false,
      inProgress: false,
      error: 'Drive API 403',
      errorKind: 'other' as const,
      lastSyncedAt: '2024-01-01T00:00:00.000Z',
      seq: 1
    }

    handlerFor(EVENTS.syncStatus)(status)

    expect(useApp.getState().sync).toEqual(status)
  })

  it('replaces the previous status rather than merging into it', () => {
    handlerFor(EVENTS.syncStatus)({ ...initialApp.sync, pending: true, seq: 1 })
    handlerFor(EVENTS.syncStatus)({ ...initialApp.sync, configured: true, seq: 2 })

    const { pending, configured } = useApp.getState().sync
    expect(pending).toBe(false)
    expect(configured).toBe(true)
  })

  // The probe and the event are two routes for one fact, and a probe taken
  // just before a transition can resolve after the event it emitted. The
  // backend's sequence number, not arrival order, decides which is newer.
  it('is not put back by a probe that read the state before it', () => {
    handlerFor(EVENTS.syncStatus)({ ...initialApp.sync, inProgress: true, seq: 3 })

    setApp({ ...appStatusDefault(), sync: { ...initialApp.sync, seq: 2 } })
    expect(useApp.getState().sync.inProgress).toBe(true)

    // A probe that saw the later state does land.
    setApp({
      ...appStatusDefault(),
      sync: { ...initialApp.sync, lastSyncedAt: '2024-01-01T00:00:00.000Z', seq: 4 }
    })
    expect(useApp.getState().sync.inProgress).toBe(false)
    expect(useApp.getState().sync.lastSyncedAt).toBe('2024-01-01T00:00:00.000Z')
  })

  it('takes the same transition again, so a configured that changed without one lands', () => {
    handlerFor(EVENTS.syncStatus)({ ...initialApp.sync, seq: 5 })
    setApp({ ...appStatusDefault(), sync: { ...initialApp.sync, configured: true, seq: 5 } })
    expect(useApp.getState().sync.configured).toBe(true)
  })
})

// Whether Rust hears the OS locking changes after the launch probe: the
// listener comes up on a thread of its own, and can be lost later.
describe('oslock:status', () => {
  it('follows the listener coming up, then being lost, after the probe', () => {
    setApp(appStatusDefault())
    expect(useApp.getState().osLock.supported).toBe(false)

    handlerFor(EVENTS.oslockStatus)({ supported: true, seq: 1 })
    expect(useApp.getState().osLock.supported).toBe(true)

    handlerFor(EVENTS.oslockStatus)({ supported: false, seq: 2 })
    expect(useApp.getState().osLock.supported).toBe(false)
  })

  it('is not put back by a probe that read it before the change', () => {
    handlerFor(EVENTS.oslockStatus)({ supported: true, seq: 1 })

    setApp({ ...appStatusDefault(), osLock: { supported: false, seq: 0 } })

    expect(useApp.getState().osLock.supported).toBe(true)
  })

  // The listener came up after the probe was read but before anything was
  // listening: its event went unheard. Asking once listening is what lands it.
  it('asks once listening, for a change no one was listening for', async () => {
    setApp(appStatusDefault())
    mockCommand('os_lock_status', () => ({ supported: true, seq: 1 }))
    clearCalls('os_lock_status')

    subscribeToEvents()

    await vi.waitFor(() => expect(useApp.getState().osLock.supported).toBe(true))
    expect(calls('os_lock_status')).toHaveLength(1)
  })

  // That answer and an event can cross on the way: the newer wins either way.
  it('keeps a later event over an earlier answer that lands after it', async () => {
    const answer = deferred<{ supported: boolean; seq: number }>()
    mockCommand('os_lock_status', () => answer.promise)
    clearCalls('os_lock_status')
    subscribeToEvents()
    await vi.waitFor(() => expect(calls('os_lock_status')).toHaveLength(1))

    handlerFor(EVENTS.oslockStatus)({ supported: false, seq: 2 })
    answer.resolve({ supported: true, seq: 1 })
    // A macrotask: every microtask the answer's delivery takes has run.
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(useApp.getState().osLock).toEqual({ supported: false, seq: 2 })
  })
})

describe('vault:locked', () => {
  it('shows the Touch ID button when a key is enrolled, not a hardcoded false', async () => {
    gate(true)
    flowMain()

    handlerFor(EVENTS.vaultLocked)({ epoch: 1 })
    expect(useApp.getState().flow).toBe('auth')

    // The regression: this used to be `flowAuth(false)` unconditionally, so an
    // in-session lock (autolock, tray) never offered Touch ID again until a
    // full app restart. The lock screen reads the gate off the re-run probe,
    // which lands after the routing rather than before it.
    await vi.waitFor(() =>
      expect(useApp.getState().status?.biometric.available).toBe(true)
    )
  })

  // The probe is async; the routing must not wait on it. While it did, the main
  // shell stayed mounted over an emptied store — live chords and all.
  it('leaves the main shell before the probe answers', () => {
    flowMain()
    setEntries([meta('a')])

    handlerFor(EVENTS.vaultLocked)({ epoch: 1 })

    expect(useApp.getState().flow).toBe('auth')
    expect(useVault.getState().items).toEqual([])
  })

  it('drops the session data with the key', async () => {
    flowMain()
    setEntries([meta('a')])
    setArchive([meta('t')])
    setCurrentEntry('a')
    setView('archive')

    handlerFor(EVENTS.vaultLocked)({ epoch: 1 })
    await vi.waitFor(() => expect(useApp.getState().flow).toBe('auth'))

    // Nothing of the unlocked vault survives the lock — the next unlock must
    // not open onto the previous session's rows.
    const vault = useVault.getState()
    expect(vault.items).toEqual([])
    expect(vault.archive).toEqual([])
    expect(vault.currentId).toBeNull()
    expect(useUi.getState().view).toBe('items')
  })

  it('lands on the plain lock screen when the probe never answered', async () => {
    mockCommand('app_status', () => Promise.reject({ kind: 'other', message: 'no backend' }))
    useApp.setState({ status: null })
    flowMain()

    handlerFor(EVENTS.vaultLocked)({ epoch: 1 })
    await vi.waitFor(() => expect(useApp.getState().flow).toBe('auth'))
    // A failed re-probe keeps the last known answer, and there was none.
    expect(useApp.getState().status).toBeNull()
  })

  // The response and the lock's event travel apart. A lock that landed after
  // an open — the OS locking a moment after the password went in — can reach
  // the webview before the open's result does; the result names the session it
  // opened, the event where the lock left the count, and the result the lock
  // overtook is not entered, whatever brought it.
  it('turns away a result the lock overtook, though it arrives after the lock', async () => {
    handlerFor(EVENTS.vaultLocked)({ epoch: 5 })

    const entered = await enterMain({ entries: [meta('a')], syncConfigured: false, session: 4 })

    expect(entered).toBe(false)
    expect(useApp.getState().flow).toBe('auth')
    expect(useVault.getState().items).toEqual([])
  })

  it('enters a result opened after the lock', async () => {
    handlerFor(EVENTS.vaultLocked)({ epoch: 5 })

    const entered = await enterMain({ entries: [meta('a')], syncConfigured: false, session: 6 })

    expect(entered).toBe(true)
    expect(useApp.getState().flow).toBe('main')
  })

  // The sequence as the review put it, through the workspace switch: Rust
  // opens the next workspace, the OS locks, the lock's event lands, and only
  // then does the switch's own answer.
  it('keeps a switch whose result a lock overtook on the lock screen', async () => {
    flowMain()
    const answer = deferred<{ entries: EntryMeta[]; syncConfigured: boolean; session: number }>()
    mockCommand('workspace_select', () => answer.promise)

    const switching = switchWorkspace('w2')
    handlerFor(EVENTS.vaultLocked)({ epoch: 8 })
    answer.resolve({ entries: [meta('secret')], syncConfigured: false, session: 7 })
    await switching

    expect(useApp.getState().flow).toBe('auth')
    expect(useVault.getState().items).toEqual([])
  })
})

// Every lock — this command, the autolock, the tray, a workspace switch — ends
// in `session::lock` on the Rust side and is announced as `vault:locked`. The
// event is what the frontend acts on, so asking and reacting are no longer two
// copies of the same three lines in three places.
describe('lockVault', () => {
  it('asks the backend and lets the event it answers with do the rest', async () => {
    flowMain()
    setEntries([meta('a')])

    await lockVault()
    await vi.waitFor(() => expect(useApp.getState().flow).toBe('auth'))

    expect(calls('lock')).toHaveLength(1)
    expect(useVault.getState().items).toEqual([])
  })

  it('leaves the screen alone when the backend never announced the lock', async () => {
    flowMain()
    setEntries([meta('a')])
    // The command resolving is not the lock happening: a backend that answered
    // without emitting has not sealed anything, and the rows stay on screen.
    mockCommand('lock', () => undefined)

    await lockVault()
    await Promise.resolve()

    expect(useApp.getState().flow).toBe('main')
    expect(useVault.getState().items.map(e => e.id)).toEqual(['a'])
  })
})
