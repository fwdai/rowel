import { useCallback, useEffect, useRef, useState } from 'react'
import type { Attachment } from '@/api/types'
import { readAttachment, renderAttachment } from '@/api/attachments'

export interface AttachmentUrl {
  /** An object URL for the file's bytes, once read; null before and on failure. */
  url: string | null
  /** The same bytes, for a view that reads them rather than points at them. */
  bytes: Uint8Array<ArrayBuffer> | null
  /** The read itself failed: a locked vault, a file gone meanwhile. */
  failed: boolean
  /** A picture neither the webview nor the backend could decode. */
  unsupported: boolean
  /**
   * The webview could not decode the picture at `url`: swap it for the
   * backend's PNG rendition of the same file. Asked twice, it gives up and
   * marks the file `unsupported`.
   */
  degrade: () => void
}

type Held = Omit<AttachmentUrl, 'degrade'> & {
  /** Which bytes `url` points at: the file's own, or the PNG made of them. */
  rendition: boolean
}

const EMPTY: Held = { url: null, bytes: null, failed: false, unsupported: false, rendition: false }

/**
 * One attachment's bytes as something the DOM can point at: an `<img>`, an
 * `<iframe>`, a thumbnail. Read when `enabled` first turns true and held as
 * long as the caller is mounted — the object URL is revoked on the way out, so
 * nothing outlives the view that asked.
 *
 * What an `<img>` makes of the bytes is the engine's business, and engines
 * differ (WebKit reads a TIFF, Chromium does not), so the caller reports a
 * picture that would not decode through `degrade` and the hook fetches the
 * backend's PNG of it in its place. One rendition per file: a PNG that will
 * not decode either is a file that cannot be shown.
 */
export function useAttachmentUrl(attachment: Attachment, enabled: boolean): AttachmentUrl {
  const [held, setHeld] = useState<Held>(EMPTY)
  // Whether the view that asked is still there, and the URL it holds — kept
  // out of state so the cleanup and a late `degrade` see the latest.
  const alive = useRef(false)
  const current = useRef<string | null>(null)

  const hold = useCallback(
    (bytes: Uint8Array<ArrayBuffer>, rendition: boolean) => {
      if (!alive.current) return
      if (current.current) URL.revokeObjectURL(current.current)
      const type = rendition ? 'image/png' : attachment.mime
      current.current = URL.createObjectURL(new Blob([bytes], { type }))
      setHeld({ ...EMPTY, url: current.current, bytes, rendition })
    },
    [attachment.mime]
  )

  useEffect(() => {
    if (!enabled) return
    alive.current = true
    readAttachment(attachment.id)
      .then(bytes => hold(bytes, false))
      .catch(() => alive.current && setHeld({ ...EMPTY, failed: true }))
    return () => {
      alive.current = false
      if (current.current) URL.revokeObjectURL(current.current)
      current.current = null
      setHeld(EMPTY)
    }
  }, [attachment.id, enabled, hold])

  const degrade = useCallback(() => {
    const giveUp = () => alive.current && setHeld(h => ({ ...h, unsupported: true }))
    if (!held.url) return
    if (held.rendition) return giveUp()
    renderAttachment(attachment.id)
      .then(bytes => hold(bytes, true))
      .catch(giveUp)
  }, [attachment.id, held.url, held.rendition, hold])

  const { url, bytes, failed, unsupported } = held
  return { url, bytes, failed, unsupported, degrade }
}
