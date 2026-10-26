import type { UnlistenFn } from '@tauri-apps/api/event'
import { on, EVENTS } from '@/api/events'
import {
  setSyncStatus,
  setupAccountPending,
  setupAccountProbed,
  setupAccountFailed,
  setRemoteVaults,
  clearSession,
  refreshApp,
  showLockScreen,
  fileOpened,
  useApp
} from './app'
import { setEntries, loadArchive, runAudit } from './vault'
import {
  useUi,
  showNotice,
  askBrowser,
  askPasskey,
  askGrant,
  closeGrantAsk,
  browserClientsChanged,
  agentsChanged
} from './ui'
import { orgChanged, orgConnectReported } from './org'
import { t } from '@/i18n'
import { providerName } from '@/api/provider'
import { syncProvider } from '@/api/sync'

// A merge can add or drop tombstones as readily as live entries, but the Archive
// only loads on entering the view — so an open Archive would sit stale until the
// user navigated away and back. Anywhere else there is nothing on screen to
// correct, and the next visit refetches anyway.
const refreshOpenArchive = () => {
  if (useUi.getState().view === 'archive') void loadArchive()
}

/**
 * A vault from the account arrived as a workspace on its own (the password
 * that opened this vault opened it too). The workspace list lives on the launch
 * probe's answer, so it is re-probed; the user is told in passing, since
 * nothing on screen changed.
 */
export const workspaceAdded = (name: string) => {
  void refreshApp()
  // The open workspace's account is the one autojoin read it from.
  const provider = syncProvider(useApp.getState().sync) ?? 'googleDrive'
  showNotice(
    t('Added “{{name}}” from your {{provider}} account', { name, provider: providerName(provider) })
  )
}

// Wires backend events to store actions. Returns a cleanup function.
export const subscribeToEvents = (): (() => void) => {
  const pending: Promise<UnlistenFn>[] = [
    on(EVENTS.syncStatus, setSyncStatus),
    // A merge brought in entries from another device: refresh the list, and the
    // audit with it — the new rows have no strength or breach result yet.
    on(EVENTS.vaultMerged, payload => {
      setEntries(payload.entries)
      void runAudit()
      refreshOpenArchive()
    }),
    // The first run's own consent flow: the same pending/result/error trio as
    // sync, against an account there is no vault behind yet.
    on(EVENTS.setupAccountPending, () => setupAccountPending()),
    on(EVENTS.setupAccountProbed, payload => setupAccountProbed(payload.files)),
    on(EVENTS.setupAccountError, payload => setupAccountFailed(payload.error)),
    on(EVENTS.workspacesRemote, payload => setRemoteVaults(payload.files)),
    on(EVENTS.workspacesAdded, payload => workspaceAdded(payload.name)),
    // This workspace was renamed on another device. The probe is what carries
    // names, so re-reading it is the whole reaction — as it is after a rename
    // made here (`RenameRow`).
    on(EVENTS.workspacesRenamed, () => void refreshApp()),
    // The one reaction to a lock, whoever asked for it: the lock command, the
    // inactivity autolock, the tray, a workspace switch. Every one of them ends
    // in `session::lock` on the Rust side, so none of them has to hand-roll
    // this — and none of them can forget half of it.
    on(EVENTS.vaultLocked, () => {
      clearSession()
      void showLockScreen()
    }),
    // A backup double-clicked while the app is up. The one opened *with* the
    // app is collected separately, once this is listening (see `App.tsx`).
    on(EVENTS.fileOpened, payload => fileOpened(payload.path)),
    // An extension asking to be let in; the dialog in `Main` answers it.
    on(EVENTS.browserAssociate, askBrowser),
    // A page's passkey ceremony, through that extension; `Main` answers it too.
    on(EVENTS.browserPasskey, askPasskey),
    // One let in: Settings › Browser extension re-reads its list.
    on(EVENTS.browserClients, () => browserClientsChanged()),
    // An agent asking to use a login, through that extension; `Main` answers it.
    on(EVENTS.agentGrant, askGrant),
    // That ask refused behind the dialog's back (a switch went off): close it.
    on(EVENTS.agentGrantClosed, payload => closeGrantAsk(payload.id)),
    // A rule or a grant came or went: Settings › Browser & agents re-reads.
    on(EVENTS.agentsChanged, () => agentsChanged()),
    on(EVENTS.orgConnect, orgConnectReported),
    on(EVENTS.orgChanged, () => orgChanged())
  ]

  return () => {
    pending.forEach(p => p.then(unlisten => unlisten()).catch(() => {}))
  }
}
