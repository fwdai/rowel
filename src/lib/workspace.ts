import type { TFunction } from 'i18next'
import type { Workspace } from '@/api/types'
import type { Provider } from '@/api/provider'
import type { RemotePack } from '@/api/setup'
import { syncDeletedRemotely, syncProvider, type SyncStatus } from '@/api/sync'

/**
 * The workspace every install starts with, and the one whose key opens the
 * app: every other workspace's key is kept sealed under it on the device, and
 * the biometric enrollment stores it. A literal the backend agrees on rather
 * than "the first in the list": the list is ordered for display, and the
 * active one is identified by id everywhere else.
 */
export const PRIMARY_WORKSPACE = 'default'

/**
 * What to call a workspace on screen. The primary one has no name until someone
 * gives it one — it predates the whole idea — so it falls back to a word for
 * "the vault you already had".
 */
export const workspaceLabel = (workspace: Workspace, t: TFunction): string =>
  workspace.name ?? t('Personal')

/**
 * Where a workspace syncs to as the registry knows it, which is all there is
 * for a locked one: the token file the backend found beside it when the list
 * was built. A bare `synced` is a backend from before there was a choice,
 * which only spoke to Google Drive.
 */
export const registeredProvider = (workspace: Workspace): Provider | null =>
  workspace.provider ?? (workspace.synced === true ? 'googleDrive' : null)

/**
 * The cloud account a workspace has a copy in, or null when it has none: what
 * Settings › Lockboxes says of each lockbox, and what the connected account's
 * card counts, so the two cannot disagree. The open one is answered by its
 * live connection — not once the account's copy is gone (deleted from another
 * device); a locked one by the connection the backend found in its own
 * directory when the list was built (`registeredProvider`) — not by its vault
 * id, which a disconnect leaves behind.
 *
 * A provider, not an account: the registry knows which provider a locked
 * lockbox's token file is for, not whose account it signs in to, so two
 * lockboxes on Google Drive may be in two people's Drives.
 *
 * An org lockbox is null here whatever its connection says: the copy on its
 * Rowel Server is the company's, not an account of the user's, so nothing
 * that follows from "has a copy in an account" — deleting it everywhere, say
 * — is theirs to do from this device.
 */
export const workspaceProvider = (
  workspace: Workspace,
  { active, sync }: { active: string; sync: SyncStatus }
): Provider | null =>
  workspace.server
    ? null
    : workspace.id === active
      ? syncDeletedRemotely(sync)
        ? null
        : syncProvider(sync)
      : registeredProvider(workspace)

/**
 * Where a lockbox's copies are, as the connected account's page tells them
 * apart: on this device and in the account (`both`), on this device alone
 * (`device`), or in the account alone (`account`).
 */
export type AccountStanding = 'both' | 'device' | 'account'

export type AccountLockbox =
  | { standing: 'both' | 'device'; workspace: Workspace; open: boolean }
  | { standing: 'account'; pack: RemotePack }

const RANK: Record<AccountStanding, number> = { both: 0, device: 1, account: 2 }

/**
 * The lockboxes as the open lockbox's cloud account sees them: what its page
 * lists, and — those `inAccount` — what its card counts, so the figure and the
 * list cannot disagree. Empty with no account connected.
 *
 * One on this device is the account's when `workspaceProvider` names the
 * account's provider, and nobody's when it names none. One in another
 * provider's account, or an org lockbox (the company server's), is not this
 * account's business and is left out. The packs the last sync run found in the
 * account and not here (`remote`, the store's `remoteVaults`) are the
 * account's own listing: the store keeps them no longer than the connection
 * that reported them, so none here is a previous account's.
 *
 * The open one first, since the connection is its own; then the rest in the
 * account, those on this device alone, and those not on it yet.
 */
export const accountLockboxes = (
  workspaces: Workspace[],
  remote: RemotePack[],
  { active, sync }: { active: string; sync: SyncStatus }
): AccountLockbox[] => {
  const provider = syncProvider(sync)
  if (!provider) return []
  const here = workspaces.flatMap((workspace): AccountLockbox[] => {
    if (workspace.server) return []
    const home = workspaceProvider(workspace, { active, sync })
    const standing = home === provider ? 'both' : home === null ? 'device' : null
    return standing ? [{ standing, workspace, open: workspace.id === active }] : []
  })
  const there = remote.map((pack): AccountLockbox => ({ standing: 'account', pack }))
  const rank = (lockbox: AccountLockbox) =>
    lockbox.standing !== 'account' && lockbox.open ? -1 : RANK[lockbox.standing]
  return [...here, ...there].sort((a, b) => rank(a) - rank(b))
}

/** Whether the account holds a copy of it: the lockboxes its card counts. */
export const inAccount = (lockbox: AccountLockbox): boolean => lockbox.standing !== 'device'
