import { create } from 'zustand'
import type { EntryType } from '@/api/types'
import type { SshKeyPair } from '@/api/tools'
import type { AssociateAsk } from '@/api/browser'
import { loadArchive, setNoEntry, useVault, selectCurrent } from './vault'

/**
 * The surfaces open over the vault and how the list is narrowed. Everything
 * here is session data — a lock drops it with the rows (`clearSession`): a
 * share dialog's link is live credentials that must not stay on screen behind
 * whoever unlocks next. Nothing that outlives a session belongs here, which is
 * why what the *platform* can do (scanning) is read off the launch probe.
 */

// `tags` is the vault by one tag (`filterTag`): every item carrying it, from
// across the vault, with the Tags tile lit. Entered through `showTag`.
export type View = 'items' | 'favorites' | 'health' | 'archive' | 'tags'

// The Settings sections, in nav order.
export type Section =
  | 'sync'
  | 'security'
  | 'audit'
  | 'import'
  | 'language'
  | 'browser'
  | 'workspaces'

/** Why a scan produced no fields. The copy for each lives in `Scan/Status`. */
export type ScanError = 'unreadable' | 'unsupported' | 'failed'

// Receives the generated value when the user confirms. The login form supplies
// one to fill its password field; the standalone ⌘G flow leaves it null and the
// dialog just copies.
export type GeneratorApply = (value: string) => void

// The SSH counterpart: a key is three draft fields, not one string, so the ssh
// editor's Generate button takes the whole pair. Opening this way also picks
// the dialog's mode — there is nothing else to generate from that row.
export type SshApply = (pair: SshKeyPair) => void

export interface Generator {
  open: boolean
  apply: GeneratorApply | null
  ssh: SshApply | null
}

export interface UiState {
  view: View
  // `null` is "All Items"; a kind is only ever open in the items view.
  filterType: EntryType | null
  // Only the Tags view holds one.
  filterTag: string | null
  query: string
  palette: boolean
  settings: boolean
  settingsSection: Section
  /**
   * Settings may not be closed or moved off its section: an operation on the
   * open section has claimed something the backend will not give back until it
   * lands (a workspace restore holds the pending Google account), so leaving
   * would only look like backing out. Set by the operation, not by the shell.
   */
  settingsLocked: boolean
  addPicker: boolean
  generator: Generator
  /** The entry being shared, or null when the send dialog is closed. */
  sendFor: string | null
  receiveOpen: boolean
  /**
   * File ids of shares that were published after their dialog had moved on,
   * whose revoke has not yet succeeded. A seal that lands late is a live link
   * in the sender's Drive that no screen ever showed; it stays here until it
   * is taken back, and every share surface retries on the way in.
   */
  orphans: string[]
  /**
   * A request to put the caret in the list's search field, as a tick rather
   * than a flag: ⌘F pressed twice running is two requests, and a flag that is
   * already raised is not a second one. `ListColumn/Search` watches it and
   * nothing ever lowers it.
   */
  searchFocus: number
  // Image scanning: the current run. Whether the platform can scan at all is
  // the probe's answer, not session state (`useScanSupported` in `store/app`).
  scanBusy: boolean
  scanError: ScanError | null
  // The app-level "Copied to Clipboard" pill is up. Raised by `flashCopied`
  // and lowered by it alone, so nothing else has to know the timing.
  copied: boolean
  /**
   * A sentence worth a moment of the user's attention and nothing more — a
   * vault the account held arriving as a workspace on its own, say. Shown by
   * `NoticeToast` for a few seconds, then gone; `null` between notices.
   */
  notice: string | null
  /**
   * A browser extension asking to connect, while its consent dialog is up. A
   * second ask replaces the first: Rust holds one at a time and gives up on
   * it by itself after a minute.
   */
  browserAsk: AssociateAsk | null
  /**
   * How many times the open vault's list of let-in extensions has changed
   * behind the frontend's back — a consent dialog answered while Settings ›
   * Browser extension is open. That section re-reads its status when this
   * moves; nothing renders the number itself.
   */
  browserClientsSeq: number
}

const GENERATOR_CLOSED: Generator = { open: false, apply: null, ssh: null }

const COPIED_TIMEOUT = 2000

// Module-level rather than in the state: the pill's countdown is not something
// anything renders, and keeping it out means `flashCopied` is the only writer
// of `copied`.
let copiedTimer: ReturnType<typeof setTimeout>

export const initialUi: UiState = {
  view: 'items',
  filterType: null,
  filterTag: null,
  query: '',
  palette: false,
  settings: false,
  // General, the first item in the nav. A deep link (the sync chip, an opened
  // backup) says otherwise for its own opening.
  settingsSection: 'language',
  settingsLocked: false,
  addPicker: false,
  generator: GENERATOR_CLOSED,
  sendFor: null,
  receiveOpen: false,
  orphans: [],
  searchFocus: 0,
  scanBusy: false,
  scanError: null,
  copied: false,
  notice: null,
  browserAsk: null,
  browserClientsSeq: 0
}

export const useUi = create<UiState>()(() => initialUi)

// --- transient notices -----------------------------------------------------------

const NOTICE_TIMEOUT = 5000
let noticeTimer: ReturnType<typeof setTimeout>

// A second notice while one is up replaces it and restarts the clock, so the
// later one gets its full few seconds.
export const showNotice = (text: string) => {
  clearTimeout(noticeTimer)
  noticeTimer = setTimeout(() => useUi.setState({ notice: null }), NOTICE_TIMEOUT)
  useUi.setState({ notice: text })
}

// --- clipboard feedback ----------------------------------------------------------

// A copy while the pill is still up restarts its two seconds rather than
// letting the first copy's timer take it down early.
export const flashCopied = () => {
  clearTimeout(copiedTimer)
  copiedTimer = setTimeout(() => useUi.setState({ copied: false }), COPIED_TIMEOUT)
  useUi.setState({ copied: true })
}

// --- views and filters -----------------------------------------------------------

// A tag belongs to the Tags view and a kind to All Items; leaving drops both.
const enterView = (view: View, patch: Partial<UiState> = {}) => {
  useUi.setState({ view, filterTag: null, filterType: null, ...patch })
  setNoEntry()
  // Tombstones are not part of the unlock payload, so the Archive reads them
  // when it is opened. Refetching on every visit is also what keeps it honest
  // after a sync merged a peer's deletes.
  if (view === 'archive') void loadArchive()
}

export const setView = (view: View) => enterView(view)

export const showTag = (tag: string) => enterView('tags', { filterTag: tag })

export const setFilterQuery = (query: string) => useUi.setState({ query })

// ⌘F comes off the window, where there is no ref to the field. Asked of the
// store rather than of the DOM: the field is one of ours, so what it should be
// doing is state like everything else on this screen.
export const focusSearch = () =>
  useUi.setState(state => ({ searchFocus: state.searchFocus + 1 }))

// A selection the new filter still shows is kept — narrowing to the kind you
// are already reading shouldn't close it. Only a selection the filter would
// hide is dropped, along with any in-progress new/edit.
const dropHidden = (hidden: (current: { type: EntryType; tags: string[] }) => boolean) => {
  const current = selectCurrent(useVault.getState())
  if (current && hidden(current)) setNoEntry()
}

export const setFilterType = (type: EntryType | null) => {
  useUi.setState({ filterType: type })
  if (type) dropHidden(current => current.type !== type)
}

export const setFilterTag = (tag: string | null) => {
  useUi.setState({ filterTag: tag })
  if (tag) dropHidden(current => !current.tags.includes(tag))
}

// --- overlays -----------------------------------------------------------------------

/**
 * The modal surfaces on screen right now, by the id each registered under
 * (`hooks/useDialogPresence`, which every dialog frame calls through the
 * shared focus hook).
 *
 * Window- and document-level accelerators (the detail pane's bare ⏎, the
 * editor's Esc and ⌘⏎, every ⌘ chord in `useShortcuts`) have to stand down
 * while something modal is up, or they fire *behind* it — dismissing a dialog
 * would also discard the draft underneath. Drops are the same question: the
 * open modal may have a drop zone of its own (Settings › Import).
 *
 * Presence rather than a list of this store's own fields: a prompt a component
 * keeps in its own state (the env editor's replace-or-merge question) is as
 * modal as a Settings dialog, and a list here would not know about it. Kept out
 * of `useUi` so a lock's reset cannot zero it under frames that are still
 * mounted and about to unregister themselves.
 */
const useDialogs = create<{ open: ReadonlySet<string> }>()(() => ({ open: new Set() }))

export const registerDialog = (id: string) =>
  useDialogs.setState(state => ({ open: new Set(state.open).add(id) }))

export const unregisterDialog = (id: string) =>
  useDialogs.setState(state => {
    const open = new Set(state.open)
    open.delete(id)
    return { open }
  })

/** Whether a modal surface owns the keyboard and the window, for an event handler. */
export const isModalOpen = () => useDialogs.getState().open.size > 0

export const openPalette = () => useUi.setState({ palette: true })
export const closePalette = () => useUi.setState({ palette: false })

// Without a section the modal reopens where it was left, so a deep link from
// the palette is the only thing that moves it.
export const openSettings = (section?: Section) =>
  useUi.setState(state => ({
    palette: false,
    settings: true,
    settingsSection: section ?? state.settingsSection
  }))
// Both refuse while locked, and refuse *here* rather than in the shell: the X,
// Escape, the backdrop, the rail tile and the palette all end up in these two,
// and a lock that any of them could walk around is not one.
export const closeSettings = () =>
  useUi.setState(state => (state.settingsLocked ? {} : { settings: false }))
export const setSettingsSection = (section: Section) =>
  useUi.setState(state => (state.settingsLocked ? {} : { settingsSection: section }))
/** Hold Settings on its current section until `lockSettings(false)`. */
export const lockSettings = (locked: boolean) => useUi.setState({ settingsLocked: locked })

export const openAddPicker = () => useUi.setState({ palette: false, addPicker: true })
export const closeAddPicker = () => useUi.setState({ addPicker: false })

export const openGenerator = (apply?: GeneratorApply) =>
  useUi.setState({ generator: { ...GENERATOR_CLOSED, open: true, apply: apply ?? null } })
export const openSshGenerator = (ssh: SshApply) =>
  useUi.setState({ generator: { ...GENERATOR_CLOSED, open: true, ssh } })
export const closeGenerator = () => useUi.setState({ generator: GENERATOR_CLOSED })

// --- sharing -----------------------------------------------------------------------

export const openSend = (entryId: string) => useUi.setState({ sendFor: entryId })
export const closeSend = () => useUi.setState({ sendFor: null })
export const openReceive = () => useUi.setState({ receiveOpen: true })
export const closeReceive = () => useUi.setState({ receiveOpen: false })

export const queueOrphan = (fileId: string) =>
  useUi.setState(state => ({
    orphans: state.orphans.includes(fileId) ? state.orphans : [...state.orphans, fileId]
  }))
export const dropOrphan = (fileId: string) =>
  useUi.setState(state => ({ orphans: state.orphans.filter(id => id !== fileId) }))

// --- browser extension -------------------------------------------------------------

export const askBrowser = (ask: AssociateAsk) => useUi.setState({ browserAsk: ask })
export const closeBrowserAsk = () => useUi.setState({ browserAsk: null })
export const browserClientsChanged = () =>
  useUi.setState(state => ({ browserClientsSeq: state.browserClientsSeq + 1 }))

// --- scanning ---------------------------------------------------------------------

// A new run replaces the previous run's complaint: the user is answering it.
export const scanStarted = () => useUi.setState({ scanBusy: true, scanError: null })
export const scanFinished = (error?: ScanError | null) =>
  useUi.setState({ scanBusy: false, scanError: error ?? null })
export const dismissScan = () => useUi.setState({ scanError: null })

export const resetUi = () => useUi.setState(initialUi, true)
