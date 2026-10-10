import { pickFileToRead } from '@/api/tools'
import { t } from '@/i18n'
import { runScan } from './run'

/**
 * Choose an image and scan it — the picked-file twin of dropping a photo on
 * the window, and on a phone the "from the library" half of the source sheet
 * (the camera half is `capture.ts`).
 *
 * The dialog is opened in Rust, which also clears the picked path for the read
 * that follows (see `pickFileToRead`), and which owns the filter: all image
 * types, which is exactly what makes iOS show the Photos picker rather than the
 * Files browser, since the dialog plugin switches to `PHPickerViewController`
 * when no filter names a non-media type (`tauri-plugin-dialog` 2.7.2,
 * `ios/Sources/DialogPlugin.swift:82-114`). Only its name is ours to translate.
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
