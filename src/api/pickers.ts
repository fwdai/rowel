import { open } from '@tauri-apps/plugin-dialog'
import { t } from '@/i18n'

interface Filter {
  /** Shown in the native dialog's own chrome, so it is translated like the rest of it. */
  name: string
  extensions: string[]
}

/**
 * One file off disk, or null when the dialog was dismissed.
 *
 * For files the *frontend* hands straight to a command that names its own
 * source (a backup to restore, an export to import). A file the backend is
 * asked to read and give back goes through `pickFileToRead` in `@/api/tools`
 * instead, which opens the dialog in Rust so the choice is one it witnessed.
 */
const pickFile = async (filters?: Filter[]): Promise<string | null> => {
  const path = await open({ multiple: false, directory: false, filters })
  return typeof path === 'string' ? path : null
}

export const pickBackup = (): Promise<string | null> =>
  pickFile([{ name: t('Rowel backup'), extensions: ['rowel'] }])

/**
 * Either backup a user may reach for from Settings › Import: a legacy
 * `.swftx`, which merges into the open vault, or a `.rowel`, which is offered
 * so the import can say where it restores instead of hiding it from the dialog.
 */
export const pickImportBackup = (): Promise<string | null> =>
  pickFile([{ name: t('Rowel backup'), extensions: ['rowel', 'swftx'] }])

export const pickImportFile = (): Promise<string | null> =>
  pickFile([{ name: t('Password exports'), extensions: ['json', 'csv', '1pux'] }])

// No extension filter: a `.env` has none, and `.env.production` is not `.production`.