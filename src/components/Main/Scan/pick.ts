import { pickFileToRead } from '@/api/tools'
import { t } from '@/i18n'
import { scanFinished } from '@/store'
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
 * Resolves to whether the window is being handed over — a file picked, or a
 * picker that failed — so a caller dismisses itself only then, and stays up
 * over a cancelled picker. Never throws: the outcome of the scan, and the
 * failure of a picker, is the status in the store (see `run.ts`), the one
 * place the user hears of either once the caller has gone.
 */
export const pickAndScan = async (browse = false): Promise<boolean> => {
  let path: string | null
  try {
    path = await pickFileToRead('image', t('Images'), browse)
  } catch {
    scanFinished('failed')
    return true
  }
  if (path === null) return false
  void runScan(path)
  return true
}
