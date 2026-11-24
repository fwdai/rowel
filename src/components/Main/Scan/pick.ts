import { pickFileToRead, type PickedFile } from '@/api/tools'
import { t } from '@/i18n'
import { scanFinished, stampSession } from '@/store'
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
 * place the user hears of either once the caller has gone. A picker that
 * answers after a lock answers a session that is gone: nothing is scanned or
 * reported, and nothing is handed over (see `stampSession`).
 */
export const pickAndScan = async (browse = false): Promise<boolean> => {
  const current = stampSession()
  let file: PickedFile | null
  try {
    file = await pickFileToRead('image', t('Images'), browse)
  } catch {
    if (!current()) return false
    scanFinished('failed')
    return true
  }
  if (file === null || !current()) return false
  void runScan(file.path)
  return true
}
