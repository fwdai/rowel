import { pickFileToRead } from '@/api/tools'
import { t } from '@/i18n'
import { runScan } from './run'

/**
 * Choose an image and scan it — the picked-file twin of dropping a photo on
 * the window, and on a phone the "Photo Library" and "Choose File" rows of the
 * source sheet (the camera row is `capture.ts`).
 *
 * The dialog is opened in Rust, which also clears the picked path for the read
 * that follows (see `pickFileToRead`), and which owns the filter: all image
 * types, which is what makes a phone show its photo library — or, with
 * `browse`, its file browser under the same filter. Only the filter's name is
 * ours to translate.
 *
 * Resolves to whether a file was picked, so a caller can dismiss itself only
 * when the window is actually being handed over. Never throws: a dialog that
 * could not open reads as nothing picked, and the outcome of the scan itself is
 * the status in the store (see `run.ts`).
 */
export const pickAndScan = async (browse = false): Promise<boolean> => {
  const path = await pickFileToRead('image', t('Images'), browse).catch(() => null)
  if (path === null) return false
  void runScan(path)
  return true
}
