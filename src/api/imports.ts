import { call } from './client'
import type { EntryMeta } from './types'

// Explicit format names understood by the backend; 'auto' detects by
// extension/content.
export type ImportFormat =
  | 'auto'
  | 'bitwarden'
  | 'cxf'
  | 'enpass'
  | 'csv'
  | 'chrome'
  | 'lastpass'
  | 'keepass'

export type ExportFormat = 'bitwarden' | 'cxf' | 'csv'

export interface RowError {
  row: number
  message: string
}

export interface ImportReport {
  total: number // parsed entries (would-be import on a dry run)
  imported: number // entries written (0 on a dry run)
  skipped: number // rows that failed to parse
  duplicates: number // rows identical to an entry already in the vault
  dryRun: boolean
  errors: RowError[]
  /** The refreshed live list after a real run; empty on a dry run. */
  entries: EntryMeta[]
}

export const importEntries = (
  path: string,
  format: ImportFormat,
  dryRun: boolean
): Promise<ImportReport> => call('import_entries', { path, format, dryRun })

export const exportEntries = (
  format: ExportFormat,
  path?: string
): Promise<string | null> => call('export_entries', { path: path ?? null, format })

// A `.env` file read whole as text, for the env kind's drop zone and picker.
// The backend refuses anything over 1 MiB or not UTF-8; the body is the secret,
// so it is handed straight to a draft and never logged.
export interface EnvFile {
  fileName: string
  body: string
}

export const readEnvFile = (path: string): Promise<EnvFile> =>
  call('read_env_file', { path })
