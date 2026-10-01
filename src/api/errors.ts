import { t } from '@/i18n'

/**
 * Every command rejection from Rust arrives as one of these. The `kind` is what
 * screens branch on — the message is for the user and may be shown as is, but
 * matching on its text is how the same failure used to read differently on two
 * screens the moment Rust reworded it.
 */
export type BackendErrorKind =
  | 'invalidPassword'
  | 'vaultTooNew'
  | 'tooManyAttempts'
  | 'locked'
  | 'staleSession'
  | 'notFound'
  | 'cancelled'
  | 'syncNotConfigured'
  | 'unsupported'
  | 'unrecognized'
  | 'shareLinkInvalid'
  | 'shareExpired'
  | 'shareTooNew'
  | 'shareTooLarge'
  | 'entryTooLargeToShare'
  | 'shareNotOwned'
  | 'shareNeedsSync'
  | 'setupBusy'
  | 'syncBusy'
  | 'driveNotConnected'
  | 'noRemoteVault'
  | 'alreadySetUp'
  | 'primaryWorkspaceOnly'
  | 'workspaceNameRequired'
  | 'workspacePasswordRequired'
  | 'lastWorkspace'
  | 'vaultAlreadyOpen'
  | 'vaultNotInAccount'
  | 'vaultDeletedRemotely'
  | 'fileTooLarge'
  | 'fileNotText'
  | 'vaultFull'
  | 'io'
  | 'serde'
  | 'crypto'
  | 'other'

export interface BackendError {
  kind: BackendErrorKind
  message: string
  /** Only on `tooManyAttempts`: seconds until another attempt is accepted. */
  retryAfterSecs?: number
}

export const isBackendError = (error: unknown): error is BackendError =>
  typeof error === 'object' &&
  error !== null &&
  typeof (error as BackendError).kind === 'string' &&
  typeof (error as BackendError).message === 'string'

export const errorKind = (error: unknown): BackendErrorKind | null =>
  isBackendError(error) ? error.kind : null

/**
 * A rejection as something showable. Empty when there is nothing worth
 * repeating, so a caller can fall back to its own wording with `||`.
 */
export const messageOf = (error: unknown): string =>
  isBackendError(error) ? error.message : error instanceof Error ? error.message : ''

/**
 * A rejection as copy the user can read in their own language. Rust's `message`
 * is English source text, so anything with a stable meaning to a user is said
 * here instead of repeated from the wire. `io`, `serde`, `crypto` and `other`
 * carry a path, a parser position or a driver's words — diagnostics, not copy —
 * so those keep the raw message, as does anything that is not a `BackendError`.
 *
 * `unsupported` keeps its message too, for a different reason: Rust builds it
 * per call site ("a login whose only secret is a passkey cannot be shared"), and
 * a translated "not supported here" would throw away the only part the user can
 * act on.
 */
export const describeError = (error: unknown): string => {
  if (!isBackendError(error)) return messageOf(error)

  switch (error.kind) {
    case 'invalidPassword':
      return t('Incorrect Master Password')
    case 'vaultTooNew':
      return t('Vault needs a newer version of the app')
    case 'tooManyAttempts':
      return t('Too many failed attempts. Try again in {{seconds}}s', {
        seconds: error.retryAfterSecs ?? 0
      })
    case 'locked':
      return t('Vault is locked')
    case 'staleSession':
      return t('The vault changed while this was running. Try again.')
    case 'notFound':
      return t('Item not found')
    case 'cancelled':
      return t('Cancelled')
    case 'syncNotConfigured':
      return t('Sync is not set up')
    case 'unrecognized':
      return t('Nothing recognized')
    case 'shareLinkInvalid':
      return t('This is not a valid share link')
    case 'shareExpired':
      return t('This share has expired or was revoked')
    case 'shareTooNew':
      return t('This share was made by a newer version of the app')
    case 'shareTooLarge':
      return t('This share is too large to open')
    case 'entryTooLargeToShare':
      return t('This entry is too large to share. Shorten its note or fields.')
    case 'shareNotOwned':
      return t('This share was created in another workspace')
    case 'shareNeedsSync':
      return t('Sync this workspace once before sharing from it')
    case 'setupBusy':
      return t('Another setup step is still running')
    case 'syncBusy':
      return t('Wait for the sync in progress to finish')
    case 'driveNotConnected':
      return t('Connect a Google account first')
    case 'noRemoteVault':
      return t('This Google account has nothing to restore')
    case 'alreadySetUp':
      return t('This device is already set up')
    case 'primaryWorkspaceOnly':
      return t('Unlock the primary workspace first')
    case 'workspaceNameRequired':
      return t('A workspace needs a name')
    case 'workspacePasswordRequired':
      return t('A workspace needs a master password')
    case 'lastWorkspace':
      return t('This is the only workspace on this device')
    case 'vaultAlreadyOpen':
      return t('This vault is already a workspace on this device')
    case 'vaultNotInAccount':
      return t('This Google account holds other vaults')
    case 'vaultDeletedRemotely':
      return t('This vault was deleted from Google Drive on another device.')
    case 'fileTooLarge':
      return t('This file is too large')
    case 'fileNotText':
      return t('This file is not text')
    case 'vaultFull':
      return t("This vault's attachments are at their limit ({{limit}})", { limit: '128 MB' })
    default:
      return messageOf(error)
  }
}

export const isTooManyAttempts = (
  error: unknown
): error is BackendError & { retryAfterSecs: number } =>
  errorKind(error) === 'tooManyAttempts' &&
  typeof (error as BackendError).retryAfterSecs === 'number'
