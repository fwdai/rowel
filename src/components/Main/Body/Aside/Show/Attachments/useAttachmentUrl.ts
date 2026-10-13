import { useCallback, useEffect, useRef, useState, type RefObject } from 'react'
import type { Attachment } from '@/api/types'
import { readAttachment, renderAttachment, thumbnailAttachment } from '@/api/attachments'
import { typeOf } from './kind'

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
 * One attachment's bytes as something the DOM can point at — an `<img>`, an
 * `<iframe>` — for the file that is open. Read when `enabled` first turns
 * true and held as long as the caller is mounted: the object URL is revoked
 * on the way out, so nothing outlives the view that asked, and only the open
 * file is ever held whole (a row draws `useThumbnail`'s small picture).
 *
 * What an `<img>` makes of the bytes is the engine's business, and engines
 * differ (WebKit reads a TIFF, Chromium does not), so the caller reports a
 * picture that would not decode through `degrade` and the hook fetches the
 * backend's PNG of it in its place. One rendition per file: a PNG that will
 * not decode either is a file that cannot be shown.
 */
export function useAttachmentUrl(attachment: Attachment, enabled: boolean): AttachmentUrl {
  const [held, setHeld] = useState<Held>(EMPTY)
  // Which file is being asked for, counted up on every change of file (and
  // on unmount), and the URL held for it — kept out of state so the cleanup
  // and a late answer see the latest. Every read and rendition carries the
  // generation it was started under and is dropped if that has moved on:
  // the view stays mounted while the preview steps from one file to the
  // next, and a slow read of the first file landing after the second's had
  // started would otherwise show the first file under the second's name.
  const generation = useRef(0)
  const current = useRef<string | null>(null)
  const type = typeOf(attachment)

  const hold = useCallback(
    (bytes: Uint8Array<ArrayBuffer>, rendition: boolean, asked: number) => {
      if (asked !== generation.current) return
      if (current.current) URL.revokeObjectURL(current.current)
      current.current = URL.createObjectURL(
        new Blob([bytes], { type: rendition ? 'image/png' : type })
      )
      setHeld({ ...EMPTY, url: current.current, bytes, rendition })
    },
    [type]
  )

  useEffect(() => {
    if (!enabled) return
    // The same counter, under a name the cleanup may read: it is a box, not a
    // node, so the lint's worry about a stale `.current` does not apply.
    const asks = generation
    const asked = ++asks.current
    readAttachment(attachment.id)
      .then(bytes => hold(bytes, false, asked))
      .catch(() => asked === asks.current && setHeld({ ...EMPTY, failed: true }))
    return () => {
      // Anything still on its way answers a question no longer asked.
      asks.current++
      if (current.current) URL.revokeObjectURL(current.current)
      current.current = null
      setHeld(EMPTY)
    }
  }, [attachment.id, enabled, hold])

  const degrade = useCallback(() => {
    const asked = generation.current
    const giveUp = () =>
      asked === generation.current && setHeld(h => ({ ...h, unsupported: true }))
    if (!held.url) return
    if (held.rendition) return giveUp()
    renderAttachment(attachment.id)
      .then(bytes => hold(bytes, true, asked))
      .catch(giveUp)
  }, [attachment.id, held.url, held.rendition, hold])

  const { url, bytes, failed, unsupported } = held
  return { url, bytes, failed, unsupported, degrade }
}

/**
 * A picture's row thumbnail: the backend's small PNG of it, as an object URL,
 * or null while it is on its way and for good if the backend cannot make one
 * (HEIC) — the row then shows the kind's glyph. Bounded on the wire and in
 * memory: a list of images used to be the files themselves, held whole, up to
 * the vault's entire budget just by opening the entry. Asked for only once
 * `row` has scrolled into view, so a long list decodes what is looked at —
 * and, with the backend decoding one picture at a time, never a burst.
 */
export function useThumbnail(
  attachment: Attachment,
  enabled: boolean,
  row: RefObject<HTMLElement | null>
): string | null {
  const [url, setUrl] = useState<string | null>(null)
  const [seen, setSeen] = useState(false)

  useEffect(() => {
    const el = row.current
    if (!enabled || seen || !el) return
    // No observer (an old engine, the test runner): the row counts as seen.
    if (typeof IntersectionObserver === 'undefined') return setSeen(true)
    const observer = new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting)) setSeen(true)
    })
    observer.observe(el)
    return () => observer.disconnect()
  }, [enabled, seen, row])

  useEffect(() => {
    if (!enabled || !seen) return
    let alive = true
    let held: string | null = null
    thumbnailAttachment(attachment.id)
      .then(bytes => {
        if (!alive) return
        held = URL.createObjectURL(new Blob([bytes], { type: 'image/png' }))
        setUrl(held)
      })
      .catch(() => {})
    return () => {
      alive = false
      if (held) URL.revokeObjectURL(held)
      setUrl(null)
    }
  }, [attachment.id, enabled, seen])

  return url
}
