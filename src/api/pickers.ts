import { t } from '@/i18n'
import { pickFileToRead } from './tools'

/**
 * The files Rowel takes in whole: a backup to restore, an export to import.
 * One file off disk, or null when the dialog was dismissed.
 *
 * The dialog opens in Rust (`pickFileToRead`), like every other pick of a file
 * the backend then reads: the path is granted for that one purpose before it
 * comes back here, and the restore or import refuses a path it was not.
 *
 * The first run's restore takes a `.rowel` backup or a legacy `.swftx` vault;
 * the extension says which command the restore step calls.
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

export const pickImportFile = (): Promise<string | null> =>
  pickFileToRead('import', t('Password exports'))
