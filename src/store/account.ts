import { setupConnect, setupDisconnect } from '@/api/setup'
import { workspaceConnect } from '@/api/workspace'
import { syncConnect } from '@/api/sync'
import type { Provider } from '@/api/provider'
import {
  useApp,
  setupAccountPending,
  setupAccountFailed,
  setupAccountReset
} from './app'
import { describeError } from '@/api/errors'

/**
 * Connecting an account for a vault that does not exist yet.
 *
 * Two screens do this — the first run, and Settings › Workspaces restoring one
 * of the account's other vaults — and they differ in one thing: which command
 * starts the consent, since the backend refuses onboarding's once a vault is on
 * disk. Everything else is shared, including the `setupAccount` slice the
 * probe's answer lands in, because only one of the two flows can ever be on
 * screen.
 */

type Connect = (provider: Provider) => Promise<void>

/**
 * The store is moved to `pending` here rather than waiting for
 * `setup:account:pending`: the press has to change the screen in the same
 * frame, and the event only confirms what this already said. A rejection is an
 * immediate failure (no OAuth client, no browser) and lands in the same place
 * `setup:account:error` would.
 */
const start = (connect: Connect, provider: Provider): void => {
  setupAccountPending(provider)
  connect(provider).catch((error: unknown) => setupAccountFailed(describeError(error)))
}

// The flow's own provider. Every way into a switch passes through a press that
// set it; Google Drive is only the answer for a slice nothing has touched.
const current = (): Provider => useApp.getState().setupAccount.provider ?? 'googleDrive'

/** Sign in as somebody else with the same provider: the old tokens go first. */
const switchAccount = (connect: Connect): void => {
  const provider = current()
  setupAccountPending(provider)
  setupDisconnect()
    .catch(() => {})
    .then(() => connect(provider))
    .catch((error: unknown) => setupAccountFailed(describeError(error)))
}

/** Open `provider`'s consent flow for a device that has no data yet. */
export const connectAccount = (provider: Provider): void => start(setupConnect, provider)

/** The same, for a device that has data and wants another of the account's. */
export const connectWorkspaceAccount = (provider: Provider): void =>
  start(workspaceConnect, provider)

export const switchSetupAccount = (): void => switchAccount(setupConnect)

export const switchWorkspaceAccount = (): void => switchAccount(workspaceConnect)

/**
 * Connect the open vault (Settings › Sync). Marked pending at the press for the
 * same reason as the others, which is also what tells the card whose name to
 * wait on. A rejection has already been reported on `sync:status`, so it only
 * puts the slice back.
 */
export const connectSync = (provider: Provider): void => {
  setupAccountPending(provider)
  syncConnect(provider).catch(setupAccountReset)
}

/** Drop the pending tokens and everything the probe said about them. */
export const forgetAccount = (): void => {
  setupAccountReset()
  setupDisconnect().catch(() => {})
}
