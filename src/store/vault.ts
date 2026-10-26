import { create } from 'zustand'
import type { Entry, EntryMeta, EntryType } from '@/api/types'
import {
  saveEntry as saveEntryCmd,
  deleteEntry as deleteEntryCmd,
  listDeleted,
  restoreEntry as restoreEntryCmd,
  purgeEntry as purgeEntryCmd,
  setFavorite,
  clearPasswordHistory as clearPasswordHistoryCmd
} from '@/api/vault'
import { addAttachment, addAttachmentBytes, deleteAttachment } from '@/api/attachments'
import { getAudit, type Audit } from '@/api/tools'
import type { EntryDraft } from '@/kinds/draft'
import { usePrefs } from './prefs'
import { useUi, setFilterType, setView } from './ui'
import { scheduleSync } from './app'

/**
 * What the unlocked vault put on screen: the rows, which one is selected and
 * whether it is being written, plus the audit of them. All of it is session
 * data — `clearSession` wipes it on lock so the next unlock (of this or any
 * other vault) never opens onto the previous one's rows.
 */
export interface VaultState {
  items: EntryMeta[]
  // Tombstones, as the Archive view lists them. Not part of the unlock payload,
  // so this stays empty until the Archive is opened.
  archive: EntryMeta[]
  /**
   * The selected row, by id across live rows and tombstones alike. While a
   * batch is picked it is only where the keyboard sits: nothing is open.
   */
  currentId: string | null
  /**
   * The rows picked for a batch action (⌘-click, ⇧-click, ⌘A): two or more,
   * or none at all — one row picked is just the selection.
   */
  picked: ReadonlySet<string>
  /** Where a ⇧ range grows from, once it is not the selection itself. */
  anchor: string | null
  /** The selected row is open in the editor. */
  editing: boolean
  /** The kind of a new entry being drafted, or null when none is. */
  creating: EntryType | null
  // Field values waiting for an editor to take them (what a scan read out of an
  // image). Consumed once — `useDraft` folds them into the draft and clears
  // them, so nothing is left to seed the next entry with.
  prefill: Record<string, string> | null
  audit: Audit | null
  /**
   * Bumped whenever what the vault holds may have changed: every `setEntries`
   * (our writes, a sync merge, an import) and every attachment write. A view
   * that keeps a copy of its own — an entry's files — rereads on it.
   */
  revision: number
}

const NONE: ReadonlySet<string> = new Set()

export const initialVault: VaultState = {
  items: [],
  archive: [],
  currentId: null,
  picked: NONE,
  anchor: null,
  editing: false,
  creating: null,
  prefill: null,
  audit: null,
  revision: 0
}

export const useVault = create<VaultState>()(() => initialVault)

// --- selectors ---------------------------------------------------------------

const findEntry = (state: Pick<VaultState, 'items' | 'archive'>, id: string | null) =>
  id === null
    ? null
    : (state.items.find(e => e.id === id) ?? state.archive.find(e => e.id === id) ?? null)

// The selection is an id, so the row it names is always the current one: a
// merge that renamed it shows the new title, one that dropped it shows nothing.
// A batch has no current entry, so everything that acts on the open one — ⌘E,
// the copy chords, a dropped file, the palette — stands down while it is up.
export const selectCurrent = (state: VaultState): EntryMeta | null =>
  selectBatch(state) ? null : findEntry(state, state.currentId)

/**
 * A batch is up. The one test of it, for every surface that changes with it —
 * the bar, the detail pane, the list's padding, the phone's push — so none can
 * think a batch is up while another has let it go.
 */
export const selectBatch = (state: VaultState): boolean => state.picked.size > 0

export const useCurrentEntry = () => useVault(selectCurrent)

// --- selection and drafting ----------------------------------------------------

const NO_BATCH = { picked: NONE, anchor: null }

const NOTHING_OPEN = { currentId: null, editing: false, creating: null, prefill: null, ...NO_BATCH }

export const setNoEntry = () => useVault.setState(NOTHING_OPEN)

// Also what a save lands on: selecting the row just written is the same state
// change as selecting any other row.
export const setCurrentEntry = (id: string) =>
  useVault.setState({ currentId: id, creating: null, editing: false, ...NO_BATCH })

export const newEntry = (type: EntryType, prefill?: Record<string, string>) =>
  useVault.setState({
    creating: type,
    editing: false,
    currentId: null,
    prefill: prefill ?? null,
    ...NO_BATCH
  })

// --- batch selection -----------------------------------------------------------

// Finder's grammar, which every desktop list shares. Each takes `order`, the
// rows as the list draws them: a range is a run of what is on screen, and a
// selection a query has hidden is never swept into a batch.

// What is picked before this gesture: the batch, or else the open row.
const pickedIn = (state: VaultState, order: readonly string[]): string[] =>
  order.filter(id => (selectBatch(state) ? state.picked.has(id) : id === state.currentId))

// Two rows or more are a batch, with the keyboard on `cursor`; one is simply
// that row selected; none is nothing open.
const settle = (picked: Set<string>, cursor: string, anchor: string | null) => {
  if (picked.size > 1) return { picked, currentId: cursor, anchor, editing: false, creating: null }
  const [only] = picked
  return only ? { currentId: only, editing: false, creating: null, ...NO_BATCH } : NOTHING_OPEN
}

/** ⌘-click: put a row into the batch, or take it out. */
export const togglePicked = (id: string, order: readonly string[]) =>
  useVault.setState(state => {
    const picked = new Set(pickedIn(state, order))
    if (!picked.delete(id)) picked.add(id)
    return settle(picked, id, id)
  })

/**
 * ⇧-click and ⇧↑↓: every row from the anchor to `id`. With `add` (⌘⇧-click)
 * the run joins what is already picked instead of replacing it. The anchor
 * stays put, so a second ⇧-click re-draws the run from the same row.
 */
export const pickRange = (id: string, order: readonly string[], add = false) =>
  useVault.setState(state => {
    const anchor = state.anchor ?? state.currentId
    const from = anchor === null ? -1 : order.indexOf(anchor)
    const to = order.indexOf(id)
    if (from === -1 || to === -1) return settle(new Set([id]), id, null)
    const run = order.slice(Math.min(from, to), Math.max(from, to) + 1)
    return settle(new Set([...(add ? pickedIn(state, order) : []), ...run]), id, anchor)
  })

/** ⌘A: every row the list shows. The keyboard stays where it was. */
export const pickAll = (order: readonly string[]) =>
  useVault.setState(state => {
    if (order.length === 0) return {}
    const { currentId } = state
    const cursor = currentId !== null && order.includes(currentId) ? currentId : order[0]
    return settle(new Set(order), cursor, state.anchor)
  })

/**
 * Hold the batch to the rows the list shows, whatever took the others out of
 * view: a query, a filter, a sync that unstarred or retagged them, a delete.
 * The list calls it each time what it shows changes (List/Manager), so a batch
 * is always a pick out of what is on screen — one row left is simply
 * selected, none is nothing open.
 */
export const keepPicked = (order: readonly string[]) =>
  useVault.setState(state => {
    const kept = new Set(order.filter(id => state.picked.has(id)))
    if (kept.size === state.picked.size) return {}
    const { currentId, anchor } = state
    const cursor = currentId !== null && order.includes(currentId) ? currentId : [...kept][0]
    return settle(kept, cursor, anchor !== null && order.includes(anchor) ? anchor : null)
  })

/**
 * End the batch (Esc, the bar's close) on the row the keyboard is on, opening
 * it: the list keeps its place rather than dropping back to nothing.
 */
export const clearPicked = () =>
  useVault.setState(state => (selectBatch(state) ? NO_BATCH : {}))

export const setPrefill = (prefill: Record<string, string>) => useVault.setState({ prefill })
export const clearPrefill = () => useVault.setState({ prefill: null })

// A tombstone has no editable form: `reveal_entry` refuses deleted rows, so an
// edit would open a pane that can never load its own values and whose save
// would resurrect the entry behind the user's back. Refusing here rather than
// at each entry point means ⌘E, the read header, the palette and anything
// added later all inherit it.
export const editEntry = () => {
  const current = selectCurrent(useVault.getState())
  if (!current || current.deletedAt) return
  useVault.setState({ editing: true, creating: null })
}

// Starts a new entry of `type` from anywhere (kind picker, palette command,
// a scan), leaving the audit view first — it has no editor to land the form in.
// `setView` clears any half-written draft, so it has to run before `newEntry`.
export const startEntry = (type: EntryType, prefill?: Record<string, string>) => {
  // Only All Items can hold a draft: every other view is a filtered or
  // read-only surface the new entry would immediately fall out of.
  if (useUi.getState().view !== 'items') setView('items')
  newEntry(type, prefill)
}

// --- rows ---------------------------------------------------------------------

// The list is replaced wholesale by a sync merge as well as by our own writes.
// A selection the incoming rows no longer have is dropped, and the edit with
// it: an edit is an edit *of* the selection, and the phone (whose form is a
// whole screen) would otherwise draw a form with no subject and no way back.
// A batch is held to what the list then shows by the list itself (`keepPicked`).
export const setEntries = (items: EntryMeta[]) =>
  useVault.setState(state => {
    const kept = findEntry({ items, archive: state.archive }, state.currentId) !== null
    return {
      items,
      currentId: kept ? state.currentId : null,
      editing: kept && state.editing,
      revision: state.revision + 1
    }
  })

const bumpRevision = () => useVault.setState(state => ({ revision: state.revision + 1 }))

export const setArchive = (archive: EntryMeta[]) => useVault.setState({ archive })

export const auditDone = (audit: Audit) => useVault.setState({ audit })

export const runAudit = () =>
  getAudit(usePrefs.getState().breachCheck)
    .then(auditDone)
    .catch(() => {})

const now = () => new Date().toISOString()

// Complete a draft into a full entry: existing entries keep their id/createdAt,
// new ones get a fresh id and timestamps. The backend restamps updatedAt on save.
const buildEntry = (draft: EntryDraft): Entry =>
  draft.id
    ? ({ ...draft, updatedAt: now() } as Entry)
    : ({ ...draft, id: crypto.randomUUID(), createdAt: now(), updatedAt: now() } as Entry)

// Insert or replace metadata rows in the list, keeping it a fresh array.
const upsert = (items: EntryMeta[], ...metas: EntryMeta[]): EntryMeta[] => {
  const fresh = new Map(metas.map(meta => [meta.id, meta]))
  const known = new Set(items.map(item => item.id))
  return [
    ...items.map(item => fresh.get(item.id) ?? item),
    ...metas.filter(meta => !known.has(meta.id))
  ]
}

// The core takes one row per command, so a batch is that many calls at once.
// What lands is applied whatever else fails — those rows really did change —
// and the first failure is thrown after, for the caller to report.
const each = async <T>(ids: readonly string[], op: (id: string) => Promise<T>) => {
  const results = await Promise.allSettled(ids.map(id => op(id)))
  const failure = results.find(result => result.status === 'rejected')
  return {
    done: new Set(ids.filter((_, i) => results[i].status === 'fulfilled')),
    values: results.flatMap(result => (result.status === 'fulfilled' ? [result.value] : [])),
    rethrow: () => {
      if (failure) throw failure.reason
    }
  }
}

export const saveEntry = async (draft: EntryDraft) => {
  const meta = await saveEntryCmd(buildEntry(draft))
  setEntries(upsert(useVault.getState().items, meta))
  // A save always lands somewhere the user can see it: a filter that would
  // hide the row just written is dropped rather than silently swallowing it.
  // An edit that drops the tag the Tags view is showing leaves the view too —
  // it is nothing without its tag.
  const { filterType, filterTag } = useUi.getState()
  if (filterType && filterType !== meta.type) setFilterType(null)
  if (filterTag && !meta.tags.includes(filterTag)) setView('items')
  setCurrentEntry(meta.id)
  scheduleSync()
  void runAudit()
}

// Writes an entry's tags back without an editor — the phone's picker. The
// secrets go back exactly as they were revealed, so to the core this is an
// edit that changed nothing but the tags: the row restamps, re-reveals and
// syncs as after any save. Unlike `saveEntry` it leaves the screen alone: the
// picker has closed before the write lands, and by then the user may have
// gone back or opened a draft, which a re-selection here would take away.
// The row is replaced where it is; the selection, if it is still this row,
// stays by `setEntries`'s own rule. Nothing about the audit changes.
//
// A write for a row that is no longer live is dropped, and so is the answer
// to one that was live when it left: the picker queues its saves, and one
// still waiting when the entry is archived (or a sync merge takes it) must
// not reach the core, whose save would bring the row back — `save_entry` is
// an upsert, and a tombstone saved over is a tombstone no more. One already
// out when the archive lands comes back with a row the list no longer has,
// which `upsert` would append as live again; it is let go instead.
export const saveTags = async (entry: Entry, tags: string[]) => {
  const live = () => useVault.getState().items.some(row => row.id === entry.id)
  if (!live()) return
  const meta = await saveEntryCmd(buildEntry({ ...entry, tags }))
  if (!live()) return
  setEntries(upsert(useVault.getState().items, meta))
  scheduleSync()
}

// What the UI calls "Archive": the backend op is a tombstone, so the rows only
// leave All Items — `loadArchive` still lists them.
export const deleteEntries = async (ids: readonly string[]) => {
  const { done, rethrow } = await each(ids, deleteEntryCmd)
  if (done.size) {
    setEntries(useVault.getState().items.filter(e => !done.has(e.id)))
    setNoEntry()
    scheduleSync()
    void runAudit()
  }
  rethrow()
}

export const deleteEntry = (id: string) => deleteEntries([id])

export const loadArchive = async () => {
  try {
    setArchive(await listDeleted())
  } catch {
    // Keep the last known list: a failed read is not an empty Archive, and
    // there is nothing the user can do about it from here.
  }
}

// Restore and purge both take the rows out of the Archive and leave nothing
// selected: the detail pane must not keep showing a row this view no longer has.
const dropFromArchive = (ids: ReadonlySet<string>) => {
  setArchive(useVault.getState().archive.filter(e => !ids.has(e.id)))
  setNoEntry()
}

export const restoreEntries = async (ids: readonly string[]) => {
  const { done, values, rethrow } = await each(ids, restoreEntryCmd)
  if (done.size) {
    setEntries(upsert(useVault.getState().items, ...values))
    dropFromArchive(done)
    scheduleSync()
    void runAudit()
  }
  rethrow()
}

export const restoreEntry = (id: string) => restoreEntries([id])

export const purgeEntries = async (ids: readonly string[]) => {
  const { done, rethrow } = await each(ids, purgeEntryCmd)
  if (done.size) {
    dropFromArchive(done)
    scheduleSync()
  }
  rethrow()
}

export const purgeEntry = (id: string) => purgeEntries([id])

// Stars or unstars a batch. The rows stay picked — they are all still there —
// except where unstarring takes them out of the view, as in Favorites.
export const favoriteEntries = async (ids: readonly string[], favorite: boolean) => {
  const { values, rethrow } = await each(ids, id => setFavorite(id, favorite))
  if (values.length) {
    setEntries(upsert(useVault.getState().items, ...values))
    if (!favorite && useUi.getState().view === 'favorites') setNoEntry()
    scheduleSync()
  }
  rethrow()
}

export const toggleFavorite = async (id: string) => {
  const entry = useVault.getState().items.find(e => e.id === id)
  if (!entry) return
  const meta = await setFavorite(id, !entry.favorite)
  setEntries(upsert(useVault.getState().items, meta))
  // Un-starring inside Favorites drops the row out of the view, so keeping it
  // selected would leave the detail pane on an entry the list no longer has.
  if (useUi.getState().view === 'favorites' && !meta.favorite) setNoEntry()
  else setCurrentEntry(meta.id)
  scheduleSync()
}

// The row's `updatedAt` moves, which is what makes the open entry reveal
// itself again (see `useRevealed`) — and come back without its history.
export const clearPasswordHistory = async (id: string) => {
  const meta = await clearPasswordHistoryCmd(id)
  setEntries(upsert(useVault.getState().items, meta))
  scheduleSync()
}

// --- attachments ----------------------------------------------------------------

// An entry's files are listed by the entry itself (`useAttachments`), but a
// change to them is a change to the vault like any other: published, and
// announced through `revision` so the open list rereads.

export const attachFile = async (entryId: string, path: string) => {
  await addAttachment(entryId, path)
  bumpRevision()
  scheduleSync()
}

// The same write, for a file the webview holds rather than one on disk.
export const attachBytes = async (entryId: string, name: string, bytes: Uint8Array) => {
  await addAttachmentBytes(entryId, name, bytes)
  bumpRevision()
  scheduleSync()
}

export const removeAttachment = async (id: string) => {
  await deleteAttachment(id)
  bumpRevision()
  scheduleSync()
}

export const resetVault = () => useVault.setState(initialVault, true)
