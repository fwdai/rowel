import { call } from './client'
import type { Entry, EntryMeta } from './types'

// Decrypt one entry's secret fields on demand (view/edit).
export const revealEntry = (id: string): Promise<Entry> => call('reveal_entry', { id })

// Persist one entry (upsert a single row); returns its refreshed metadata.
export const saveEntry = (entry: Entry): Promise<EntryMeta> => call('save_entry', { entry })

// Tombstone one entry.
export const deleteEntry = (id: string): Promise<void> => call('delete_entry', { id })

// The Archive: tombstoned entries' metadata, newest deletion first.
export const listDeleted = (): Promise<EntryMeta[]> => call('list_deleted')

// Un-tombstone one entry; returns its refreshed metadata.
export const restoreEntry = (id: string): Promise<EntryMeta> => call('restore_entry', { id })

// Discard a tombstoned entry's contents for good.
export const purgeEntry = (id: string): Promise<void> => call('purge_entry', { id })

// Star or unstar one entry; returns its refreshed metadata. A metadata-only
// write — the payload is never unsealed, so it costs no reveal.
export const setFavorite = (id: string, favorite: boolean): Promise<EntryMeta> =>
  call('set_favorite', { id, favorite })

// Forget a login's previous passwords; returns its refreshed metadata. The
// core clears the stored row itself — nothing is sent but the id.
export const clearPasswordHistory = (id: string): Promise<EntryMeta> =>
  call('clear_password_history', { id })

export interface SwftxImport {
  count: number
  /** The refreshed live list, so the caller never has to re-read the vault. */
  entries: EntryMeta[]
}

// Merge a `.swftx` file (encrypted under its own `password`) into the currently
// unlocked vault. Runs off the UI thread and emits `import:progress`.
export const importSwftx = (path: string, password: string): Promise<SwftxImport> =>
  call('import_swftx', { path, password })

// Save a `.rowel` backup: the vault's own encrypted snapshot, restorable on a
// fresh install via `setupRestoreFromFile`. The `password` must match the
// unlocked vault; it proves the exporter can open what they are carrying away.
export const exportVault = (password: string): Promise<string | null> =>
  call('export_vault', { password })

// Write a revealed env file back to disk through the save dialog, defaulting
// to the name it came in with (or ".env"). Resolves to the chosen path, or
// null if the dialog was dismissed. Desktop only — the caller hides the action
// on mobile.
export const saveEnvFile = (
  fileNameSuggestion: string,
  body: string
): Promise<string | null> => call('save_env_file', { fileNameSuggestion, body })
