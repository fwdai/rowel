import { call } from './client'
import type { Provider } from './provider'
import type { UnlockResult } from './types'

/**
 * Additional encrypted databases. Each has its own file and its own key, but
 * the app is unlocked as a whole: one master password (or one biometric
 * prompt) opens the primary, and every other workspace's key is kept sealed
 * under the primary's on this device, so moving between them never asks
 * again. A workspace that has never been opened with its password here is the
 * one exception — its first switch lands on its lock screen, and that unlock is
 * what seals its key for the ones after. Each carries its own account
 * connection and syncs its own pack.
 */

/**
 * Create a workspace and open it. `password` is the device's master password —
 * the primary's — which the backend proves before creating anything, so every
 * workspace stays under the one password; a wrong one rejects as
 * `invalidPassword`. It comes back already active and unlocked, so the result
 * is an unlock's, not a setup's: the caller has a session to enter main with
 * and a previous workspace's data to drop first.
 *
 * `color` is the tile's palette key (see `lib/workspaceColor`); left out, the
 * workspace has none and its tile takes the hue its id hashes to.
 */
export const workspaceCreate = (
  name: string,
  password: string,
  color?: string | null
): Promise<UnlockResult> => call('workspace_create', { name, password, color: color ?? null })

/**
 * Close whatever is open and make `id` the active workspace. Persisted, so a
 * relaunch lands on it.
 *
 * Resolves with an unlock's result when the backend holds that workspace's key
 * and opened it — the caller enters main with it, exactly as after a create.
 * Resolves `null` when it does not: the switch then ends on that workspace's
 * lock screen, announced by the `vault:locked` the backend emits for it.
 */
export const workspaceSelect = (id: string): Promise<UnlockResult | null> =>
  call('workspace_select', { id })

/** Rename a workspace. Metadata only: the vault behind it is untouched. */
export const workspaceRename = (id: string, name: string): Promise<void> =>
  call('workspace_rename', { id, name })

/**
 * Give a workspace's tile a palette key, or `null` to go back to the hue its id
 * hashes to. Kept on this device only: nothing is written to the vault and
 * nothing syncs, so a locked workspace is recoloured just as the open one is.
 */
export const workspaceSetColor = (id: string, color: string | null): Promise<void> =>
  call('workspace_set_color', { id, color })

/**
 * Let agents use items in a workspace, or stop. Kept on this device only, like
 * the colour, so a locked workspace is switched just as the open one is.
 * Turning the open one off ends every grant that is up.
 */
export const workspaceSetAgents = (id: string, enabled: boolean): Promise<void> =>
  call('workspace_set_agents', { id, enabled })

/**
 * Remove a workspace's vault from this device. `password` is that workspace's
 * own master password — a locked workspace's key is nowhere in memory, so the
 * backend derives from its descriptor and lets the database answer; a wrong one
 * rejects as `invalidPassword` and nothing is removed.
 *
 * Local by default: whatever the vault has in its account is left there, so
 * the account still holds it and it can be restored as a workspace again. With
 * `everywhere`, the vault's copy in the account goes first and a marker takes
 * its place, which is what has the account's other devices stop syncing it
 * instead of uploading theirs back — and the remote step runs before anything
 * local is removed, so a network failure leaves the workspace here to try again
 * from.
 *
 * Refused as `lastWorkspace` when it is the only workspace left. Deleting the
 * active one ends its session, so `vault:locked` arrives and the app comes back
 * on the survivor's lock screen — there is nothing to do with this promise but
 * report what went wrong.
 */
export const workspaceDelete = (
  id: string,
  password: string,
  everywhere: boolean
): Promise<void> => call('workspace_delete', { id, password, everywhere })

/**
 * Move lockbox `workspaceId` onto the vault password: the answer to a
 * `vaultPasswordRequired` rejection from an unlock or a restore, which carries
 * the id. `lockboxPassword` is the one that just opened it, `vaultPassword` the
 * one that opens the primary; each is proved under its own backoff, so a wrong
 * one rejects as `invalidPassword` (or `tooManyAttempts`). Afterwards the
 * lockbox opens with the vault password and is sealed under the app key, so a
 * switch to it opens it without asking — and its other devices are asked for
 * the vault password, as after a password change.
 */
export const workspaceAdoptVaultPassword = (
  workspaceId: string,
  lockboxPassword: string,
  vaultPassword: string
): Promise<void> =>
  call('workspace_adopt_vault_password', { workspaceId, lockboxPassword, vaultPassword })

/**
 * Connect a `provider` account for a workspace that does not exist yet. The
 * onboarding connect with its "no vault here" precondition dropped, so it
 * answers on exactly the same events: `setup:account:pending`, then one of
 * `setup:account:probed` / `setup:account:error`. Nothing comes back through
 * this promise; a rejection is a failure to start at all.
 *
 * The tokens it leaves pending are forgotten with `setupDisconnect`, which both
 * flows share.
 */
export const workspaceConnect = (provider: Provider): Promise<void> =>
  call('workspace_connect', { provider })

/**
 * Make a new workspace out of one of the connected account's vaults. `fileId`
 * is the pack the user picked from what the probe listed, `password` is that
 * vault's own master password and `name` is the label the workspace gets here.
 *
 * Resolves like an unlock of the restored vault, with `syncConfigured: true` —
 * the account is sealed under its key by the time this returns. A wrong
 * password rejects as `invalidPassword` and changes nothing, so retrying costs
 * only the typing.
 */
export const workspaceRestoreFromPending = (
  name: string,
  password: string,
  fileId: string
): Promise<UnlockResult> =>
  call('workspace_restore_from_pending', { name, password, fileId })

/**
 * The same, for one of the vaults the open workspace's own account holds and
 * this device does not (`workspaces:remote`). No sign-in: the open workspace's
 * tokens do the download, and a copy is sealed under the restored vault's key.
 * Refused as `syncNotConfigured` from a workspace that does not sync.
 */
export const workspaceRestoreFromAccount = (
  name: string,
  password: string,
  fileId: string
): Promise<UnlockResult> =>
  call('workspace_restore_from_account', { name, password, fileId })
