import type { Attachment } from '@/api/types'
import { cx } from '@/utils/cx'
import {
  FileArchiveGlyph,
  FileGlyph,
  FileImageGlyph,
  FileTextGlyph
} from '@/components/Main/icons'
import { extensionOf, previewKind, type PreviewKind } from './kind'

interface Props {
  attachment: Attachment
  /** The image itself, once read; the tile shows the file's page until then. */
  url?: string | null
  /** The engine could not decode what `url` points at (see `useAttachmentUrl`). */
  onBroken?: () => void
  /** A list row's 40px thumbnail, or the preview's 112px one for a file it cannot show. */
  size?: 'row' | 'hero'
}

const SIZE: Record<NonNullable<Props['size']>, string> = {
  row: 'size-10 rounded-lg',
  hero: 'size-28 rounded-2xl'
}

const GLYPH: Record<NonNullable<Props['size']>, number> = { row: 18, hero: 36 }

// The glyph says what kind of file it is; anything the type table could not
// place is just a file.
const GLYPHS: Record<PreviewKind, typeof FileGlyph> = {
  image: FileImageGlyph,
  pdf: FileTextGlyph,
  text: FileTextGlyph,
  none: FileGlyph
}

/**
 * What a file looks like: the picture itself for an image, and for anything
 * else a page — the kind's glyph on the tinted tile, and at the preview's
 * size the file's type stamped in the corner ("PDF"), which is how Files and
 * Finder tell one document from the next; a 40px row thumbnail has no room
 * for the stamp, and the caption beside it says the type anyway. A picture
 * the engine cannot decode is reported up (`onBroken`), and the page holds
 * until a URL it can draw arrives, or for good.
 */
export default function Thumb({ attachment, url, onBroken, size = 'row' }: Props) {
  const Glyph = attachment.mime === 'application/zip' ? FileArchiveGlyph : GLYPHS[previewKind(attachment)]
  const extension = size === 'hero' ? extensionOf(attachment.name) : null

  return (
    <span
      className={cx(
        'relative grid flex-none place-items-center overflow-hidden bg-tile text-text2',
        SIZE[size]
      )}
    >
      <Glyph size={GLYPH[size]} />
      {extension && (
        <span className="absolute bottom-1.5 left-1.5 rounded-xs border border-line bg-card px-1 py-px text-[10px] leading-[14px] font-semibold tracking-wide text-text2">
          {extension}
        </span>
      )}
      {url && (
        // Over the page rather than instead of it: an image that fails to
        // decode draws nothing, and the page is what shows through. Keyed by
        // URL so a swapped source starts a fresh load and a fresh error.
        <img
          key={url}
          src={url}
          alt=""
          data-testid="attachment-thumb"
          onError={onBroken}
          className="absolute inset-0 h-full w-full bg-tile object-cover"
        />
      )}
      {/* A hairline over the picture, so a white image still has an edge. */}
      {url && <span className="absolute inset-0 rounded-[inherit] ring-1 ring-line ring-inset" />}
    </span>
  )
}
