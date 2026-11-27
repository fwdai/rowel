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
  | 'apple'
  | 'lastpass'
  | 'keepass'
  | '1password'

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
  /** Rows that imported but left something behind; counted in `total`, never in `skipped`. */
  warnings: RowError[]
  /** The refreshed live list after a real run; empty on a dry run. */
  entries: EntryMeta[]
}

export const importEntries = (
  path: string,
  format: ImportFormat,
  dryRun: boolean
): Promise<ImportReport> => call('import_entries', { path, format, dryRun })

/**
 * Whether this device can be handed another password manager's data by the
 * system (iOS 26's "Export Data to Another App"). Nowhere else is it true.
 */
export const exchangeSupported = (): Promise<boolean> => call('exchange_supported')

/**
 * Whether such a transfer is waiting. Rust fetches and holds it, and says so
 * as `exchange:received`; this is for one that arrived before the webview was
 * listening. Asking takes nothing.
 */
export const exchangePending = (): Promise<boolean> => call('exchange_pending')

/**
 * `importEntries` for the transfer Rust holds: previewed on a dry run, written
 * and let go of otherwise. Only the report ever reaches the webview. Rejects
 * when nothing is held (a lock drops it) or the transfer itself failed.
 */
export const importExchange = (dryRun: boolean): Promise<ImportReport> =>
  call('import_exchange', { dryRun })

/**
 * Write the whole vault, in plaintext, to a file the user picks in the save
 * dialog. `password` must be the master password — the same proof the
 * encrypted `exportVault` asks for; a wrong one rejects `invalidPassword`.
 */
export const exportEntries = (
  format: ExportFormat,
  password: string,
  path?: string
): Promise<string | null> => call('export_entries', { path: path ?? null, format, password })

// A `.env` file read whole as text, for the env kind's drop zone and picker.
// The backend refuses anything over 1 MiB or not UTF-8; the body is the secret,
// so it is handed straight to a draft and never logged.
export interface EnvFile {
  fileName: string
  body: string
}

export const readEnvFile = (path: string): Promise<EnvFile> =>
  call('read_env_file', { path })
