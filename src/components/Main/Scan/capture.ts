import { scanFinished, scanStarted, useApp } from '@/store'
import { runScan } from './run'

/**
 * Scan a photo the phone's system picker handed the webview's file input
 * (`elements/FileInput`, opened by `AddSecret/ScanAction`) — from the camera,
 * the photo library or Files alike. It is a `File` with no path the backend
 * could be pointed at, so its bytes are what travels.
 *
 * The read is an await with nothing of the session in hand, so it is stamped
 * like a Rust command's: a lock while it runs (and whatever unlock follows,
 * this vault's or another workspace's) means the session that took the photo
 * is gone, and the photo is dropped rather than scanned into the one that
 * never asked — the lock already cleared the status it would have reported to.
 *
 * Resolves once the scan is under way; its outcome is the status in the store
 * (see `run.ts`). Never throws: a file that cannot be read reads as a failed
 * scan, through the same status — the picker that asked has already closed
 * by now, so the status is the only place the user would hear of it.
 */
export const scanCapture = async (file: File): Promise<void> => {
  const session = useApp.getState().locks
  scanStarted()
  let image: Uint8Array
  try {
    image = new Uint8Array(await file.arrayBuffer())
  } catch {
    if (useApp.getState().locks === session) scanFinished('failed')
    return
  }
  if (useApp.getState().locks !== session) return
  await runScan(image)
}
