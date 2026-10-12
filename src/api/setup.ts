import { call } from './client'
import type { Provider } from './provider'
import type { UnlockResult } from './types'

/**
 * One sealed `.rowel` pack a connected account already holds, as the setup
 * probe reports it. `size` is bytes; `modifiedTime` is RFC3339.
 *
 * An account can hold several — two installs syncing their own primary vault
 * to one account each mint their own vault id — so the probe lists them all and
 * the user says which is theirs. `id` is the provider's own handle for the file
 * (a Drive file id, a Dropbox `id:`), opaque here, and the only thing that names
 * that choice back to the backend.
 *
 * `name` is the file's name in `Rowel/Vaults/`: the vault's own id, not a label
 * the user chose, so it is not shown. `vaultId` is that same id parsed out —
 * always present, since a file whose name carries no id is not a pack at all.
 */
export interface RemotePack {
  id: string
  name: string
  vaultId: string
  size: number
  modifiedTime: string
  provider: Provider
}

/**
 * Start `provider`'s consent flow *before* there is anything on disk, so the
 * first run can look for data to restore. The outcome never arrives through
 * this promise: it comes back as `setup:account:pending` and then exactly one
 * of `setup:account:probed` / `setup:account:error`. A rejection here is an
 * immediate failure (no OAuth client configured) and is shown the same way an
 * error event is.
 */
export const setupConnect = (provider: Provider): Promise<void> =>
  call('setup_connect', { provider })

/** Forget the tokens `setupConnect` left pending (Go back, Switch account). */
export const setupDisconnect = (): Promise<void> => call('setup_disconnect')

/**
 * Pull the pack the user chose out of the ones the probe listed, and unseal it
 * here. Resolves `syncConfigured: true`. `fileId` is that pack's `id`; one the
 * account no longer holds fails as `noRemoteVault`.
 */
export const setupRestoreFromAccount = (
  password: string,
  fileId: string
): Promise<UnlockResult> => call('setup_restore_from_account', { password, fileId })

/**
 * Create the local data under `password`. Any tokens left pending by
 * `setupConnect` are adopted, so the result reports sync as configured.
 * Only reached with an account the probe found empty: one that already holds a
 * vault is restored instead, never added to.
 */
export const setupCreate = (password: string): Promise<UnlockResult> =>
  call('setup_create', { password })

/**
 * First run only: install a `.rowel` backup as this device's vault. The same
 * pack an account holds, so it fails the same ways as `setupRestoreFromAccount`.
 */
export const setupRestoreFromFile = (
  path: string,
  password: string
): Promise<UnlockResult> => call('setup_restore_from_file', { path, password })
