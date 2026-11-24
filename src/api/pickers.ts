import { t } from '@/i18n'
import { pickFileToRead, type PickedFile } from './tools'

/**
 * The files Rowel takes in whole: a backup to restore, an export to import.
 * One file, as its address and its name (`PickedFile`), or null when the
 * dialog was dismissed.
 *
 * The dialog opens in Rust (`pickFileToRead`), like every other pick of a file
 * the backend then reads: the path is granted for that one purpose before it
 * comes back here, and the restore or import refuses a path it was not.
 */
export const pickBackup = (): Promise<PickedFile | null> =>
  pickFileToRead('backup', t('Rowel backup'))

/**
 * Either backup a user may reach for from Settings › Import: a legacy
 * `.swftx`, which merges into the open vault, or a `.rowel`, which is offered
 * so the import can say where it restores instead of hiding it from the dialog.
 */
export const pickImportBackup = (): Promise<PickedFile | null> =>
  pickFileToRead('importBackup', t('Rowel backup'))

/**
 * A `.rowel` alone, for Settings › Lockboxes, which restores it as a lockbox
 * beside the open ones: a legacy `.swftx` is not a lockbox, and merges from
 * Settings › Import instead.
 */
export const pickLockboxBackup = (): Promise<PickedFile | null> =>
  pickFileToRead('lockboxBackup', t('Rowel backup'))

export const pickImportFile = (): Promise<PickedFile | null> =>
  pickFileToRead('import', t('Password exports'))
