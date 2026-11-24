import { t } from '@/i18n'
import { describeError } from '@/api/errors'
import { pickFile, type PickKind } from '@/api/tools'
import { showNotice } from '@/store'

/**
 * The one way the webview picks a file for the backend to read (`pickFile`).
 * A pick that fails (the provider refuses, the copy cannot be made, the file
 * is too large) is shown as a notice and resolves `null`, as a dismissed
 * dialog does, so no caller has to decide what a failure means. A pick that
 * outlived its session resolves `null` from Rust with nothing to show.
 */
export const pickFileToRead = (
  kind: PickKind,
  label?: string,
  browse = false
): Promise<string | null> =>
  pickFile(kind, label, browse).catch((error: unknown) => {
    showNotice(describeError(error))
    return null
  })

/**
 * The files Rowel takes in whole: a backup to restore, an export to import.
 * One file off disk, or null when the dialog was dismissed.
 *
 * The dialog opens in Rust (`pickFileToRead`), like every other pick of a file
 * the backend then reads: the path is granted for that one purpose before it
 * comes back here, and the restore or import refuses a path it was not.
 */
export const pickBackup = (): Promise<string | null> =>
  pickFileToRead('backup', t('Rowel backup'))

/**
 * Either backup a user may reach for from Settings › Import: a legacy
 * `.swftx`, which merges into the open vault, or a `.rowel`, which is offered
 * so the import can say where it restores instead of hiding it from the dialog.
 */
export const pickImportBackup = (): Promise<string | null> =>
  pickFileToRead('importBackup', t('Rowel backup'))

/**
 * A `.rowel` alone, for Settings › Lockboxes, which restores it as a lockbox
 * beside the open ones: a legacy `.swftx` is not a lockbox, and merges from
 * Settings › Import instead.
 */
export const pickLockboxBackup = (): Promise<string | null> =>
  pickFileToRead('lockboxBackup', t('Rowel backup'))

export const pickImportFile = (): Promise<string | null> =>
  pickFileToRead('import', t('Password exports'))
