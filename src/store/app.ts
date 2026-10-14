import { create } from 'zustand'
import type { UnlockResult } from '@/api/types'
import { appStatus, type AppStatus } from '@/api/app'
import { lock } from '@/api/auth'
import type { Provider } from '@/api/provider'
import type { RemotePack } from '@/api/setup'
import { syncNow, type SyncStatus } from '@/api/sync'
import { checkForUpdate } from '@/api/autoUpdate'
import { resetEmblems } from '@/hooks/useEmblem'
import { resetFavicons } from '@/hooks/useFavicon'
import { setEntries, resetVault, runAudit } from './vault'
import { openSettings, resetUi } from './ui'
import { hydratePrefs, hydrateFromProbe, prefsMark } from './prefs'

/**
 * State that lives as long as the app does, across locks: which flow is on
 * screen and the lock screen's gate, the backend's sync and first-run account
 * probe as it reports them, and a staged update.
 */

export type FlowName = 'setup' | 'auth' | 'main'

export type UpdateCheckStatus = 'checking' | 'uptodate' | 'error' | null

/**
 * `pending` — consent is out with the browser
 * `found`   — the probe came back with at least one pack to restore
 * `empty`   — the probe came back with nothing; the account is usable, just bare
 * `error`   — the connect or the probe failed
 * `restoring` — a pack from `found` is being unsealed into a workspace; the
 *               account is spoken for until that lands one way or the other
 */
export type SetupAccountStatus = 'idle' | 'pending' | 'found' | 'empty' | 'error' | 'restoring'

/**
 * The first run's account probe, which has no vault behind it yet — so it
 * cannot live in `sync`, whose `configured` means "this vault syncs".
 *
 * `provider` is the cloud the flow is with. The press that starts a connect
 * sets it, so every screen can name the provider before the probe answers, and
 * Try again and Switch account go back to the same one.
 *
 * `files` is every vault the account holds, newest first, because one account
 * can hold several: two installs syncing their own primary each mint a vault
 * id. `selectedId` is the one the next step acts on — restore unseals it,
 * "start fresh" archives it — defaulting to the newest until the user says
 * otherwise, so the single-vault case never asks a question it has one answer
 * to.
 */
export interface SetupAccountState {
  status: SetupAccountStatus
  provider: Provider | null
  files: RemotePack[]
  selectedId: string | null
  error: string | null
}

export interface AppState {
  flow: FlowName
  /**
   * The launch probe's last answer, in one place. It was eight `app_status`
   * calls racing each other — one per screen that wanted a leaf of it — so
   * every screen reads this instead and re-renders when it is refreshed. The
   * lock screen's gate (`biometric.available`, `biometric.type`) is read off it
   * rather than copied out at each flow change.
   *
   * Null until the boot probe lands (`boot.ts`), and on a host where that call
   * failed outright; every reader treats that as "nothing known yet".
   */
  status: AppStatus | null
  /**
   * A workspace switch is in flight: the backend may already be pointed at the
   * next workspace while `status` still describes the one being left. The lock
   * screen keeps drawing what it has — the chip and the gate change together,
   * in the render the probe lands — but takes no attempt meanwhile, since one
   * would reach a vault other than the one on screen (see `switchWorkspace`).
   */
  switching: boolean
  /**
   * The backend's sync status, verbatim. It owns every flow — it opens the
   * browser, hears back from it, runs the sync — so it is the one that can say;
   * every change arrives whole as `sync:status` and is stored as-is, and the
   * launch probe carries the same snapshot to start from (`status.sync`).
   *
   * The one place "does this vault sync" is read from. It used to be two — a
   * copy on `status` that only the probe refreshed, and this — so connecting
   * an account in Settings left the footer on the same screen saying the vault was
   * local until the next lock.
   */
  sync: SyncStatus
  setupAccount: SetupAccountState
  /**
   * The account's vaults that are not workspaces on this device, as the last
   * sync run reported them (`workspaces:remote`). Settings › Workspaces offers
   * to restore them. Dropped with the session: they are the open workspace's
   * account's, and the next unlock's sync says again.
   */
  remoteVaults: RemotePack[]
  /**
   * How many sessions have been dropped (`clearSession`): a stamp for work in
   * the webview that awaits with nothing of the session in hand — a photo being
   * read for a scan — so it can tell, on resuming, that the session that asked
   * is gone and the one now open (this vault's or another workspace's) never
   * asked for it. The Rust side keeps the same check around its own awaits.
   */
  locks: number
  /**
   * A backup the OS asked the app to open (`file:opened`), waiting for a
   * screen that can take it: the restore step on a fresh install, Settings ›
   * Import once a vault is open. Kept across a lock — the vault has to be
   * unlocked before anything can be imported into it — and cleared by the
   * screen that takes it (`claimOpenedFile`).
   */
  openedFile: string | null
  update: {
    // Version + release notes of a staged update awaiting restart (null = none).
    readyVersion: string | null
    readyNotes: string | null
    // Transient status of a manual update check (null = idle).
    status: UpdateCheckStatus
  }
}

const ACCOUNT_IDLE: SetupAccountState = {
  status: 'idle',
  provider: null,
  files: [],
  selectedId: null,
  error: null
}

export const initialApp: AppState = {
  // The auth screen is the default; setup is reached explicitly via `flowSetup`
  // once the probe says there is nothing on disk. With `status` null nothing
  // biometric is drawn before the probe answers.
  flow: 'auth',
  status: null,
  switching: false,
  sync: {
    configured: false,
    provider: null,
    pending: false,
    inProgress: false,
    error: null,
    errorKind: null,
    lastSyncedAt: null,
    seq: 0
  },
  setupAccount: ACCOUNT_IDLE,
  remoteVaults: [],
  locks: 0,
  openedFile: null,
  update: { readyVersion: null, readyNotes: null, status: null }
}

export const useApp = create<AppState>()(() => initialApp)

// --- launch probe -----------------------------------------------------------------

/**
 * Take a probe's answer wholesale (see `boot.ts`). `sync` is lifted out of it
 * into its own slot rather than read off `status`: the probe is only one of
 * the two things that report sync, and the events that report the rest write
 * here too — so its snapshot goes through the same ordering they do.
 *
 * The settings it carries go to the prefs store every time, not only at boot:
 * a probe that answers late — the shell's own re-ask after the boot probe
 * failed — is then the first word on the stored theme and accent, and the
 * prefs store's subscriber paints the document from whatever changed.
 *
 * With a `mark` (see `prefsMark`, taken when the probe went out) the settings
 * are taken only if no preference write has overtaken the probe since; the
 * boot path passes none, since nothing can have been written before it.
 */
export const setApp = (status: AppStatus, mark?: number) => {
  useApp.setState(state => ({ status, sync: newer(state.sync, status.sync) }))
  if (mark === undefined) hydratePrefs(status.settings)
  else hydrateFromProbe(status.settings, mark)
}

/**
 * Of two sync snapshots, the one the backend produced later. The probe and the
 * `sync:status` event are two routes for the same fact, and a probe taken just
 * before a transition can resolve after the event that transition emitted;
 * without this it would put the older state back on screen. Equal sequence
 * numbers mean the same transition, and the incoming copy is taken so a
 * `configured` that changed without a transition still lands.
 */
const newer = (held: SyncStatus, incoming: SyncStatus): SyncStatus =>
  incoming.seq >= held.seq ? incoming : held

/**
 * Whether this platform has a text recognizer at all — a build-time fact about
 * the OS, so it is read off the probe rather than copied into the session's own
 * state and re-asked on every unlock.
 */
export const useScanSupported = () => useApp(state => state.status?.scanSupported ?? false)

const NO_PROVIDERS: Provider[] = []

/**
 * The providers this build can connect, as the probe reported them — a fact
 * about the build, like `scanSupported`. Nothing before the probe lands: every
 * picker sits on a screen the probe's answer routed to, so none is drawn with
 * nothing known, and an empty row is the honest fallback if one ever were.
 */
export const useProviders = () => useApp(state => state.status?.providers ?? NO_PROVIDERS)

/**
 * Ask again, after something that can change the answer: an unlock, a lock, an
 * enrollment. Resolves with what it stored — or null, keeping the last known
 * answer, if the call failed — and never rejects, so no caller has to guard it.
 */
export const refreshApp = (): Promise<AppStatus | null> => {
  // Stamped before the request leaves, so a write issued while it is out — or
  // still unanswered — outranks the settings it brings back.
  const mark = prefsMark()
  return appStatus()
    .then(status => {
      setApp(status, mark)
      return status
    })
    .catch(() => null)
}

// --- flow -----------------------------------------------------------------------

export const flowSetup = () => useApp.setState({ flow: 'setup' })
const flowAuth = () => useApp.setState({ flow: 'auth' })
export const flowMain = () => useApp.setState({ flow: 'main' })

// The lock screen reads its gate off `status`, so a lock re-runs the probe:
// whether a key is enrolled can have changed since the last one. Asked rather
// than assumed — hardcoding `false` here is how the Touch ID button used to
// vanish on every in-session lock.
//
// Routed first, probed second. The session is already gone by the time this
// runs, and waiting on the probe kept the main shell — with its live chords and
// an empty list — on screen until the answer came back. The lock screen draws
// the last known gate meanwhile and re-renders when the probe lands.
export const showLockScreen = () => {
  flowAuth()
  return refreshApp()
}

/**
 * Forget whether biometrics open the vault, for when the probe that would say
 * could not be had after a workspace switch. One enrollment serves the device,
 * but whether it opens a given workspace is that workspace's own answer (its
 * key has to be sealed under the enrolled one), so the gate last known belongs
 * to the workspace that was left. `false` is the safe default: a gate that is
 * not offered, rather than one that is offered and refused.
 *
 * Only a fallback. Clearing it up front, ahead of every switch, took the Touch
 * ID segment off the passphrase card for the round trip and put it back when
 * the probe landed — the card visibly blinked on each switch.
 */
export const forgetBiometricGate = () =>
  useApp.setState(state =>
    state.status
      ? { status: { ...state.status, biometric: { ...state.status.biometric, available: false } } }
      : {}
  )

export const setSwitching = (switching: boolean) => useApp.setState({ switching })

// Everything the unlocked session put in the stores. A lock has to drop all of
// it — it outlives the session otherwise, and the next unlock (of this or any
// other vault) opens onto the previous one's rows. Session-shaped state (flow,
// sync, update, prefs, locale) is deliberately kept.
//
// The favicon cache goes with them: it is keyed by hostname and holds the icons
// of the rows that were on screen, so leaving it would carry one vault's sites
// into the next unlock — of this vault or of another workspace. The emblem
// cache goes for the same reason, with its passports' countries.
export const clearSession = () => {
  resetVault()
  resetUi()
  resetFavicons()
  resetEmblems()
  cancelScheduledSync()
  useApp.setState(state => ({ remoteVaults: [], locks: state.locks + 1 }))
}

/** What the last sync run found in the account and not here. */
export const setRemoteVaults = (files: RemotePack[]) => useApp.setState({ remoteVaults: files })

// Manual lock, from anywhere (top chrome, Settings, palette). Nothing to do
// here but ask: the backend emits `vault:locked` for every lock there is, and
// the single reaction to it (events.ts) drops the session and shows the lock
// screen — for this, for the autolock, for the tray and for a workspace switch.
export const lockVault = () => lock().catch(() => {})

export const enterMain = async (result: UnlockResult) => {
  setEntries(result.entries)
  flowMain()
  // A backup the OS opened while the vault was locked (or before it existed)
  // has waited for this: the only screen that can import it is now reachable.
  if (useApp.getState().openedFile) openSettings('import')
  // The unlock result carries whether this vault syncs; everything else about
  // sync arrives as `sync:status` once a flow or a run happens. No re-probe:
  // `initialized` and `scanSupported` cannot have changed, and this is the one
  // answer an unlock does change.
  //
  // Which provider: the status the launch probe gave for an existing vault,
  // or — for a vault that was just created or restored behind a first-run or
  // Settings connect, which no status has described yet — the account that
  // connect was for. Without that, the gap until the first run's status read
  // as Google Drive (`syncProvider`'s fallback) and showed Drive-only controls
  // over a Dropbox vault.
  useApp.setState(state => ({
    sync: {
      ...state.sync,
      configured: result.syncConfigured,
      provider: result.syncConfigured
        ? (state.sync.provider ?? state.setupAccount.provider)
        : null
    }
  }))
  // One run on unlock: this device may have been off while another pushed,
  // and it may itself be holding writes a previous session never published.
  if (result.syncConfigured) syncNow().catch(() => {})
  void runAudit()
}

// --- a backup the OS opened with the app -------------------------------------------

/**
 * The OS handed over a backup (a double-clicked `.rowel` or `.swftx`). Where it
 * goes depends on the flow: the first run's restore step and Settings › Import
 * each watch `openedFile` and claim it when they are on screen. An unlocked
 * vault is sent to Import at once; a locked one gets there through `enterMain`.
 */
export const fileOpened = (path: string) => {
  useApp.setState({ openedFile: path })
  if (useApp.getState().flow === 'main') openSettings('import')
}

/** A screen took the file, so nothing else offers it again. */
export const claimOpenedFile = (): string | null => {
  const path = useApp.getState().openedFile
  if (path) useApp.setState({ openedFile: null })
  return path
}

// --- sync -----------------------------------------------------------------------

/**
 * How long a write waits before it is published.
 *
 * A push is the whole vault, so firing one per write would send the same
 * snapshot several times over during a run of quick edits. The timer resets on
 * every write, so a burst costs one push once it settles — and two seconds is
 * enough to settle a burst, since writes are form submits rather than
 * keystrokes. A write that lands while a run is already in flight is not lost
 * either: the backend keeps the request and runs once more when that run ends.
 * The debounce lives here rather than in the backend because the backend has
 * no write hook. A quit inside the window loses nothing: the next unlock runs
 * a sync anyway.
 */
const SYNC_DEBOUNCE_MS = 2_000

let syncTimer: ReturnType<typeof setTimeout> | undefined

// Drops a write waiting to be published. Called on lock: the backend has no key
// to push with any more, and the next unlock syncs anyway.
const cancelScheduledSync = () => {
  if (syncTimer) clearTimeout(syncTimer)
  syncTimer = undefined
}

export const scheduleSync = () => {
  if (!useApp.getState().sync.configured) return
  cancelScheduledSync()
  syncTimer = setTimeout(() => {
    syncTimer = undefined
    syncNow().catch(() => {})
  }, SYNC_DEBOUNCE_MS)
}

export const setSyncStatus = (sync: SyncStatus) =>
  useApp.setState(state => ({ sync: newer(state.sync, sync) }))

// --- first-run account probe -------------------------------------------------------

/**
 * Consent is out. From a press, `provider` says with whom; from the backend's
 * `setup:account:pending`, which names nobody, the one already held stays.
 */
export const setupAccountPending = (provider?: Provider) =>
  useApp.setState(state => ({
    setupAccount: {
      ...ACCOUNT_IDLE,
      status: 'pending',
      provider: provider ?? state.setupAccount.provider
    }
  }))
export const setupAccountProbed = (files: RemotePack[]) =>
  useApp.setState(state => ({
    setupAccount: {
      status: files.length > 0 ? 'found' : 'empty',
      provider: files[0]?.provider ?? state.setupAccount.provider,
      files,
      selectedId: files[0]?.id ?? null,
      error: null
    }
  }))
export const setupAccountFailed = (error: string) =>
  useApp.setState(state => ({
    setupAccount: {
      ...ACCOUNT_IDLE,
      status: 'error',
      provider: state.setupAccount.provider,
      error
    }
  }))
export const setupAccountReset = () => useApp.setState({ setupAccount: ACCOUNT_IDLE })

/**
 * The restore is in flight. The list and the pick stay, because a wrong
 * password comes straight back to them.
 */
export const setupAccountRestoring = () =>
  useApp.setState(state => ({ setupAccount: { ...state.setupAccount, status: 'restoring' } }))

/**
 * The restore failed and the form is back in the user's hands. Only from
 * `restoring`: the screen may have been left and reset meanwhile, and a form
 * over an emptied list would be worse than the idle row.
 */
export const setupAccountRestoreFailed = () =>
  useApp.setState(state =>
    state.setupAccount.status === 'restoring'
      ? { setupAccount: { ...state.setupAccount, status: 'found' } }
      : {}
  )

/** The user picked one of several vaults; every later step follows this. */
export const setupAccountSelect = (id: string) =>
  useApp.setState(state => ({ setupAccount: { ...state.setupAccount, selectedId: id } }))

/**
 * The vault the flow is about, for the screens that describe or act on one.
 * A selector rather than stored state: it is `files` and `selectedId` read
 * together, and there is nothing to keep in step.
 */
export const selectedRemotePack = (state: AppState): RemotePack | null =>
  state.setupAccount.files.find(file => file.id === state.setupAccount.selectedId) ?? null

// --- updates ----------------------------------------------------------------------

// Records a staged update (drives the restart toast).
export const setUpdateReady = (version: string, notes: string | null) =>
  useApp.setState(state => ({
    update: { ...state.update, readyVersion: version, readyNotes: notes }
  }))

// Dismisses the restart toast; the staged update still applies on next launch.
export const dismissUpdate = () =>
  useApp.setState(state => ({ update: { ...state.update, readyVersion: null, readyNotes: null } }))

const setUpdateStatus = (status: UpdateCheckStatus) =>
  useApp.setState(state => ({ update: { ...state.update, status } }))

// On-demand check with visible feedback for every outcome.
export const runUpdateCheck = async () => {
  if (useApp.getState().update.status === 'checking') return
  setUpdateStatus('checking')

  const result = await checkForUpdate()
  if (result.kind === 'staged') {
    useApp.setState({
      update: { readyVersion: result.version, readyNotes: result.notes, status: null }
    })
    return
  }
  // Nothing to report on a build without an updater (mobile ships through the
  // App Store); the control that starts a check is hidden there anyway.
  if (result.kind === 'unsupported') {
    setUpdateStatus(null)
    return
  }

  const status = result.kind === 'uptodate' ? 'uptodate' : 'error'
  setUpdateStatus(status)
  setTimeout(
    () => {
      if (useApp.getState().update.status === status) setUpdateStatus(null)
    },
    status === 'uptodate' ? 4000 : 6000
  )
}
