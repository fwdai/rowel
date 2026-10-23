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

/** Whether an OS path names one of our backups, by extension, case aside. */
export const isBackupFile = (path: string): boolean =>
  path.toLowerCase().endsWith(BACKUP_EXTENSION)

/** Whether an OS path names a legacy vault, by extension, case aside. */
export const isLegacyBackupFile = (path: string): boolean =>
  path.toLowerCase().endsWith(LEGACY_BACKUP_EXTENSION)

/** Either file the first run's restore step takes. */
export const isRestorableFile = (path: string): boolean =>
  isBackupFile(path) || isLegacyBackupFile(path)
