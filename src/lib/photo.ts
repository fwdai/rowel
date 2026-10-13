/**
 * The longest side a camera photo is kept at. A phone's camera takes 12 MP and
 * more (4032×3024 and up, several MB as a JPEG); a card or a document page
 * reads as well at this size — the MRZ's characters stay well over the height
 * text recognition needs — at a fraction of the bytes.
 */
export const PHOTO_MAX_SIDE = 2560

const PHOTO_QUALITY = 0.85

// A 2×1 JPEG whose EXIF says "rotate 90°": an engine that honors the flag
// decodes it as 1×2.
const ROTATED_PROBE =
  '/9j/4AAQSkZJRgABAQAAAQABAAD/4QAiRXhpZgAATU0AKgAAAAgAAQESAAMAAAABAAYAAAAAAAD/2wBDAFA3PEY8MlBGQUZaVVBfeMiCeG5uePWvuZHI////////////////////////////////////////////////////wAALCAABAAIBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAAAP/EABQQAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8AP//Z'

const DECODE: ImageBitmapOptions = { imageOrientation: 'from-image' }

let orientation: Promise<boolean> | null = null

/**
 * Whether this engine's `createImageBitmap` turns a photo upright by its EXIF
 * orientation. A phone stores most photos sideways with a flag saying so, and
 * a canvas re-encode drops the flag: an engine that ignores it would bake a
 * sideways card into the copy, which text recognition reads far worse — so
 * the answer is measured once, not assumed.
 */
const honorsOrientation = (): Promise<boolean> =>
  (orientation ??= (async () => {
    const bytes = Uint8Array.from(atob(ROTATED_PROBE), c => c.charCodeAt(0))
    const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/jpeg' }), DECODE)
    const upright = bitmap.width === 1 && bitmap.height === 2
    bitmap.close()
    return upright
  })().catch(() => false))

const encode = (canvas: HTMLCanvasElement): Promise<Blob | null> =>
  new Promise(resolve => canvas.toBlob(resolve, 'image/jpeg', PHOTO_QUALITY))

const asJpegName = (name: string) => `${name.replace(/\.[^./]*$/, '') || 'photo'}.jpg`

/**
 * A camera photo cut down to `PHOTO_MAX_SIDE` and re-encoded as a JPEG, so a
 * scan or an attachment sends — and keeps — a few hundred KB rather than
 * several MB. It is the only file the webview ever sends as bytes, and on
 * Tauri's `postMessage` fallback those bytes go as a JSON array of numbers
 * that Rust parses whole (see `commands::body_bytes`), so its size is what
 * that path costs.
 *
 * Never throws, and never makes a photo worse: the original is what comes
 * back when it is already small enough, when this engine cannot decode or
 * re-encode it, when it would not turn the copy upright, or when the copy
 * came out no smaller.
 */
export const shrinkPhoto = async (file: File): Promise<File> => {
  try {
    if (typeof createImageBitmap !== 'function' || !(await honorsOrientation())) return file
    const bitmap = await createImageBitmap(file, DECODE)
    try {
      const scale = PHOTO_MAX_SIDE / Math.max(bitmap.width, bitmap.height)
      if (scale >= 1) return file
      const canvas = document.createElement('canvas')
      canvas.width = Math.round(bitmap.width * scale)
      canvas.height = Math.round(bitmap.height * scale)
      const context = canvas.getContext('2d')
      if (!context) return file
      context.imageSmoothingQuality = 'high'
      context.drawImage(bitmap, 0, 0, canvas.width, canvas.height)
      const blob = await encode(canvas)
      // WebKit holds a canvas's pixels until it is resized away.
      canvas.width = canvas.height = 0
      if (!blob || blob.size >= file.size) return file
      return new File([blob], asJpegName(file.name), { type: 'image/jpeg' })
    } finally {
      bitmap.close()
    }
  } catch {
    return file
  }
}
