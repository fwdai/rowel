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
  /** The selected row, by id across live rows and tombstones alike. */
  currentId: string | null
  /** The selected row is open in the editor. */
  editing: boolean
  /** The kind of a new entry being drafted, or null when none is. */
  creating: EntryType | null
  // Field values waiting for an editor to take them (what a scan read out of an
  // image). Consumed once — `useDraft` folds them into the draft and clears
  // them, so nothing is left to seed the next entry with.
  prefill: Record<string, string> | null
  audit: Audit | null
}

export const initialVault: VaultState = {
  items: [],
  archive: [],
  currentId: null,
  editing: false,
  creating: null,
  prefill: null,
  audit: null
}

export const useVault = create<VaultState>()(() => initialVault)

// --- selectors ---------------------------------------------------------------

const findEntry = (state: Pick<VaultState, 'items' | 'archive'>, id: string | null) =>
  id === null
    ? null
    : (state.items.find(e => e.id === id) ?? state.archive.find(e => e.id === id) ?? null)

// The selection is an id, so the row it names is always the current one: a
// merge that renamed it shows the new title, one that dropped it shows nothing.
export const selectCurrent = (state: VaultState): EntryMeta | null =>
  findEntry(state, state.currentId)

export const useCurrentEntry = () => useVault(selectCurrent)

// --- selection and drafting ----------------------------------------------------

const NOTHING_OPEN = { currentId: null, editing: false, creating: null, prefill: null }

export const setNoEntry = () => useVault.setState(NOTHING_OPEN)

// Also what a save lands on: selecting the row just written is the same state
// change as selecting any other row.
export const setCurrentEntry = (id: string) =>
  useVault.setState({ currentId: id, creating: null, editing: false })

export const newEntry = (type: EntryType, prefill?: Record<string, string>) =>
  useVault.setState({ creating: type, editing: false, currentId: null, prefill: prefill ?? null })

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
export const setEntries = (items: EntryMeta[]) =>
  useVault.setState(state => {
    const kept = findEntry({ items, archive: state.archive }, state.currentId) !== null
    return { items, currentId: kept ? state.currentId : null, editing: kept && state.editing }
  })

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

// Insert or replace one metadata row in the list, keeping it a fresh array.
const upsert = (items: EntryMeta[], meta: EntryMeta): EntryMeta[] => {
  const index = items.findIndex(e => e.id === meta.id)
  return index === -1
    ? [...items, meta]
    : items.map((item, i) => (i === index ? meta : item))
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

// What the UI calls "Archive": the backend op is a tombstone, so the row only
// leaves All Items — `loadArchive` still lists it.
export const deleteEntry = async (id: string) => {
  await deleteEntryCmd(id)
  setEntries(useVault.getState().items.filter(e => e.id !== id))
  setNoEntry()
  scheduleSync()
  void runAudit()
}

export const loadArchive = async () => {
  try {
    setArchive(await listDeleted())
  } catch {
    // Keep the last known list: a failed read is not an empty Archive, and
    // there is nothing the user can do about it from here.
  }
}

// Restore and purge both take the row out of the Archive and leave nothing
// selected: the detail pane must not keep showing a row this view no longer has.
const dropFromArchive = (id: string) => {
  setArchive(useVault.getState().archive.filter(e => e.id !== id))
  setNoEntry()
}

export const restoreEntry = async (id: string) => {
  const meta = await restoreEntryCmd(id)
  setEntries(upsert(useVault.getState().items, meta))
  dropFromArchive(id)
  scheduleSync()
  void runAudit()
}

export const purgeEntry = async (id: string) => {
  await purgeEntryCmd(id)
  dropFromArchive(id)
  scheduleSync()
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

export const resetVault =() => useVault.setState(initialVault, true)
