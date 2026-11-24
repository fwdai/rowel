/**
 * The backup file the app writes and restores (`export_vault`,
 * `setup_restore_from_file`). The same extension `BACKUP_EXTENSION` names in
 * Rust and the desktop file association registers in `tauri.conf.json`.
 */
export const BACKUP_EXTENSION = '.rowel'

/**
 * The legacy app's vault, sealed under its own password: merged into an open
 * vault by `import_swftx`, or made a fresh install's vault by
 * `setup_restore_from_legacy`. `LEGACY_BACKUP_EXTENSION` in Rust.
 */
export const LEGACY_BACKUP_EXTENSION = '.swftx'

// Each is told from a file's name (`PickedFile.name`), never a picked file's
// address, which on Android ends in a document id. A path on disk ends in its
// name, so a dropped path may be asked directly.

/** Whether a file's name is one of our backups, by extension, case aside. */
export const isBackupFile = (name: string): boolean =>
  name.toLowerCase().endsWith(BACKUP_EXTENSION)

/** Whether a file's name is a legacy vault, by extension, case aside. */
export const isLegacyBackupFile = (name: string): boolean =>
  name.toLowerCase().endsWith(LEGACY_BACKUP_EXTENSION)

/** Either file the first run's restore step takes. */
export const isRestorableFile = (name: string): boolean =>
  isBackupFile(name) || isLegacyBackupFile(name)
