import { runScan } from './run'

/**
 * Scan a photo the phone's camera just took through the webview's capture
 * input (the "take a photo" half of `SourceSheet`). It is a `File` with no path
 * the backend could be pointed at — it was never written anywhere — so its
 * bytes are what travels.
 *
 * Resolves once the scan is under way; its outcome is the status in the store
 * (see `run.ts`). Never throws: a file that cannot be read reads as a failed
 * scan.
 */
export const scanCapture = async (file: File): Promise<void> => {
  let image: Uint8Array
  try {
    image = new Uint8Array(await file.arrayBuffer())
  } catch {
    return
  }
  await runScan(image)
}
