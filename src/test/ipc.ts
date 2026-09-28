import type { BackendError } from '@/api/errors'
import type { AppStatus, Settings } from '@/api/app'
import type { BrowserStatus } from '@/api/browser'
import { EVENTS } from '@/api/events'
import { emitEventSoon } from './events'

/**
 * The fake Rust backend. One mock of `@tauri-apps/api/core` stands in for every
 * command, keyed by the name Rust registers it under, so a new wrapper needs no
 * new mock and a renamed one fails loudly instead of silently resolving.
 */

type Args = Record<string, unknown>
type Handler = (args: Args) => unknown

const reject = (error: BackendError) => () => Promise.reject(error)

const meta = (id: string, type = 'login', title = '', favorite = false) => ({
  id,
  type,
  title,
  tags: [],
  urlHost: '',
  favorite
})

const session = { entries: [], syncConfigured: false }

// The preferences Rust would hand back on a fresh install, and the file it
// would hand them back from: `set_settings` merges into this the way the real
// command merges into `settings.json`, so a component sees its own write come
// back round. Reset with the rest of the fakes between tests.
//
// Spelled out rather than imported from `store/prefs`: this file is what the
// `@tauri-apps/api/core` mock loads, and anything in the store reaches that
// same module through `api/client` — the mock factory would be re-entered
// while it is still resolving. It is Rust's `settings.rs` both copies mirror,
// and `prefs.test` asserts the two agree, which is what caught them drifting
// by a generator flag.
const DEFAULT_SETTINGS: Settings = {
  autolockSecs: 60,
  clipboardTimeoutMs: 30000,
  dateFormat: 'MM/DD/YYYY',
  sort: 'recent',
  theme: 'light',
  accent: 'ink',
  locale: null,
  breachCheck: false,
  generator: {
    length: 20,
    numbers: true,
    symbols: true,
    uppercase: true,
    lowercase: true,
    exclude: '',
    excludeSimilarCharacters: false
  }
}

let settings: Settings = DEFAULT_SETTINGS

/**
 * The launch probe's answer on a plain desktop with nothing enrolled. Exported
 * because the store now holds it too: `renderWithStore` seeds the same object
 * `boot.ts` would have put there (see test/utils).
 */
export const appStatusDefault = (): AppStatus => ({
  initialized: true,
  version: '1.0.0',
  locale: 'en-US',
  settings,
  sync: {
    configured: false,
    pending: false,
    inProgress: false,
    error: null,
    errorKind: null,
    lastSyncedAt: null,
    seq: 0
  },
  // Off by default, so no suite sees a scan affordance it did not ask for.
  scanSupported: false,
  // The desktop's gate, and what every pre-existing spec asserts by name.
  biometric: { available: false, canEnroll: false, type: 'touch', mode: null },
  // One workspace: the ordinary install, where nothing about workspaces is on
  // screen. A suite about them seeds a second entry.
  workspaces: [{ id: 'default', name: null }],
  activeWorkspace: 'default'
})

const BROWSER_OFF: BrowserStatus = {
  enabled: false,
  browsers: [
    { id: 'chrome', label: 'Google Chrome', detected: true, installed: false, conflict: false }
  ],
  clients: []
}

const DEFAULTS: Record<string, Handler> = {
  app_status: appStatusDefault,
  app_ready: () => undefined,
  // Nothing was double-clicked to launch the app, which is every ordinary run.
  take_opened_file: () => null,

  setup: () => undefined,
  unlock: () => session,
  // Rust seals the vault and says so (`session::lock`); the frontend does
  // nothing about a lock until the event lands, so the fake has to emit it too
  // — after this promise resolves, as the real one does.
  lock: () => emitEventSoon(EVENTS.vaultLocked, undefined),
  // The idle-clock ping `Main` sends on mount and on input; nothing to answer.
  touch_activity: () => undefined,
  unlock_biometric: () => session,
  enable_biometric: () => 'protected',
  disable_biometric: () => undefined,
  change_master_password: () => undefined,

  // A new workspace arrives active and unlocked, so it answers like an unlock.
  workspace_create: () => session,
  // Onboarding's connect, reached from Settings: the probe's answer comes back
  // as `setup:drive:*`, never through this promise.
  workspace_drive_connect: () => undefined,
  // A restored one arrives the same way, but connected — its account was sealed
  // under the restored vault's key on the way in.
  workspace_restore_from_drive: () => ({ entries: [], syncConfigured: true }),
  // The same landing from the open workspace's own account: no sign-in first.
  workspace_restore_from_account: () => ({ entries: [], syncConfigured: true }),
  // A switch to a workspace Rust holds no key for: it ends as the lock it is,
  // announced like any other. (One it does hold answers like an unlock — a
  // spec that wants that path mocks the command with a session.)
  workspace_select: () => emitEventSoon(EVENTS.vaultLocked, undefined),
  workspace_rename: () => undefined,
  // Registry only in Rust, and the fake keeps no registry: the list a spec
  // sees is the `app_status` it mocks, so a recolour there is the spec's to set.
  workspace_set_color: () => undefined,

  // First run. The probe's result never comes back through these promises —
  // it arrives as `setup:drive:*`, which a spec drives through the store.
  setup_drive_connect: () => undefined,
  setup_drive_disconnect: () => undefined,
  setup_create: () => session,
  setup_restore_from_drive: () => ({ entries: [], syncConfigured: true }),
  setup_restore_from_file: () => session,

  reveal_entry: ({ id }) => ({ id, type: 'login', title: '' }),
  save_entry: ({ entry }) => {
    const { id, type, title } = entry as { id: string; type: string; title: string }
    return meta(id, type, title)
  },
  delete_entry: () => undefined,
  list_deleted: () => [],
  restore_entry: ({ id }) => meta(id as string),
  purge_entry: () => undefined,
  set_favorite: ({ id, favorite }) => meta(id as string, 'login', '', favorite as boolean),

  import_entries: ({ dryRun }) => ({
    total: 0,
    imported: 0,
    skipped: 0,
    dryRun: dryRun as boolean,
    errors: [],
    entries: []
  }),
  export_entries: () => null,
  read_env_file: reject({ kind: 'fileNotText', message: 'the file is not text' }),

  import_swftx: () => ({ count: 0, entries: [] }),
  export_vault: () => null,
  save_env_file: () => null,

  generate_password: () => 'Generated123!',
  generate_ssh_key: () => ({
    privateKey:
      '-----BEGIN OPENSSH PRIVATE KEY-----\nc2VjcmV0\n-----END OPENSSH PRIVATE KEY-----\n',
    publicKey: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI',
    fingerprint: 'SHA256:GeneratedFingerprint'
  }),
  generate_otp: () => ({ code: '123456', time: 30 }),
  get_audit: () => ({}),
  fetch_favicon: () => null,
  copy_to_clipboard: () => undefined,
  scan_image: reject({ kind: 'unrecognized', message: 'nothing recognized' }),
  // The dialog Rust opens: nothing picked unless a test says otherwise.
  pick_file: () => null,

  set_settings: ({ patch }) => {
    settings = { ...settings, ...(patch as Partial<Settings>) }
    return settings
  },

  sync_connect: () => undefined,
  sync_disconnect: () => undefined,
  sync_now: () => undefined,
  // The account taking the open vault (it was empty, or already held this
  // vault); the run it starts reports on `sync:status`, never through this
  // promise. A spec about an account holding other vaults rejects it with
  // `vaultNotInAccount` instead.
  sync_adopt_pending: () => undefined,

  // Sharing. The link is the shape the backend hands back — file id plus key —
  // and `share_open` resolves a plain login, so a suite only overrides the one
  // call it is about.
  share_create: () => ({
    link: 'rowel://share#v1.file-1.a2V5LTFrZXktMWtleS0xa2V5LTFrZXktMWtleS0xa2V5',
    fileId: 'file-1',
    expiresAt: '2024-01-02T00:00:00.000Z'
  }),
  share_open: () => ({
    id: '',
    type: 'login',
    title: 'Shared Netflix',
    website: 'https://netflix.com',
    username: 'shared@example.com',
    password: 'from-a-friend',
    email: '',
    note: '',
    otp: ''
  }),
  share_revoke: () => undefined,
  share_list: () => [],

  // The extension host, off, with nothing let in. Every change answers with
  // the status as Rust would then have it.
  browser_status: () => BROWSER_OFF,
  browser_set_enabled: ({ enabled }) => ({ ...BROWSER_OFF, enabled: enabled as boolean }),
  browser_respond: () => undefined,
  browser_forget_client: () => BROWSER_OFF
}

const overrides = new Map<string, Handler>()
const queued = new Map<string, Handler[]>()
const recorded = new Map<string, Args[]>()

/** What `@tauri-apps/api/core`'s `invoke` is replaced with (see test/setup.ts). */
export const invokeMock = (command: string, args: Args = {}): Promise<unknown> => {
  recorded.set(command, [...(recorded.get(command) ?? []), args])
  const handler =
    queued.get(command)?.shift() ?? overrides.get(command) ?? DEFAULTS[command]
  if (!handler)
    return Promise.reject({ kind: 'other', message: `no mock for ${command}` } as BackendError)
  // Called synchronously, like the mock it replaces: a spec that hands back a
  // promise it resolves by hand needs the handler to have run by the time the
  // call returns.
  try {
    return Promise.resolve(handler(args))
  } catch (error) {
    return Promise.reject(error)
  }
}

/** Stand in for one command for the rest of this test. Reset between tests. */
export const mockCommand = (command: string, impl: Handler): void => {
  overrides.set(command, impl)
}

/** Stand in for the next call only; queue several to script a sequence. */
export const mockCommandOnce = (command: string, impl: Handler): void => {
  queued.set(command, [...(queued.get(command) ?? []), impl])
}

/** Every argument object one command was invoked with, in order. */
export const calls = (command: string): Args[] => recorded.get(command) ?? []

/** Forget one command's calls, to count only what happens from here on. */
export const clearCalls = (command: string): void => {
  recorded.delete(command)
}

export const resetIpc = (): void => {
  overrides.clear()
  queued.clear()
  recorded.clear()
  settings = DEFAULT_SETTINGS
}
