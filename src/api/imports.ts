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
 * What Rust holds of another app's data: which transfer, by its arrival
 * number, or none; and `change`, the count of changes that answer has been
 * through, which is what puts two tellings of it in order. Rust is what knows:
 * it fetches the data, keeps it, lets go of it on an import or when the
 * session ends, and announces each change as `exchange:held`.
 */
export interface ExchangeHeld {
  arrival: number | null
  change: number
}

/**
 * What Rust holds now. Asked once, when the shell starts listening, for what
 * it was not there to hear. Asking takes nothing.
 */
export const exchangeHeld = (): Promise<ExchangeHeld> => call('exchange_held')

/**
 * `importEntries` for the transfer `arrival`: previewed on a dry run, written
 * and let go of otherwise. Only the report ever reaches the webview. Naming
 * the arrival is what ties an import to its preview: Rust rejects one it no
 * longer holds (replaced by a newer transfer, imported, dropped by a lock)
 * rather than answer with another's rows, and rejects with the reason when
 * the transfer itself failed.
 */
export const importExchange = (arrival: number, dryRun: boolean): Promise<ImportReport> =>
  call('import_exchange', { arrival, dryRun })

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
