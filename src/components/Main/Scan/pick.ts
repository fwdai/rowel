import { pickFileToRead } from '@/api/tools'
import { t } from '@/i18n'
import { runScan } from './run'

/**
 * Choose an image and scan it — the picked-file twin of dropping a photo on
 * the window. Desktop only: a phone goes through the system's sheet and scans
 * what it hands back from its bytes (`capture.ts`).
 *
 * The dialog is opened in Rust, which also clears the picked path for the read
 * that follows (see `pickFileToRead`), and which owns the filter: all image
 * types. Only its name is ours to translate.
 *
 * Resolves to whether a file was picked, so a caller can dismiss itself only
 * when the window is actually being handed over. Never throws: a dialog that
 * could not open reads as nothing picked, and the outcome of the scan itself is
 * the status in the store (see `run.ts`).
 */
export const pickAndScan = async (): Promise<boolean> => {
  const path = await pickFileToRead('image', t('Images')).catch(() => null)
  if (path === null) return false
  void runScan(path)
  return true
}
