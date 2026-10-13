import { useRef } from 'react'
import { useTranslation } from 'react-i18next'
import type { Attachment } from '@/api/types'
import IconButton from '@/components/elements/IconButton'
import { HOVER_ONLY, META, ROW_HAIRLINE } from '@/components/elements/tokens'
import { DownloadGlyph, TrashGlyph } from '@/components/Main/icons'
import { isMobile } from '@/lib/platform'
import { cx } from '@/utils/cx'
import { fileCaption, previewKind } from './kind'
import Thumb from './Thumb'
import { useThumbnail } from './useAttachmentUrl'

interface Props {
  attachment: Attachment
  onOpen: () => void
  onSave: () => void
  /** Ask to remove the file; the section asks and removes. */
  onRemove: () => void
}

/**
 * One attached file, as a line of a plain list: a 40px thumbnail — the
 * picture itself for an image, the kind's page for anything else — beside
 * its name over its size and type, a hairline to the next, and nothing
 * around it. The list sits on the screen's own ground, as a list of files
 * does in Mail or in Notes' attachment view; a card would make a panel of
 * what is really a short inventory, and a grid of tiles would make a gallery
 * of what is really a vault. The whole line is a press that opens the file
 * (`Preview`), where saving and removing live.
 *
 * On the desktop the two also stand at the line's end, shown as the pointer
 * reaches it and to a Tab (`HOVER_ONLY`), the way every value row offers its
 * copy: a pointer expects to save a file without opening it first, and the
 * glyphs cost the line nothing at rest. A phone has no hover, and its lines
 * carry no controls at all — the preview is the one path there.
 *
 * The desktop line lights up as a rounded tile rather than a band: it reaches
 * 8px past the content on both sides (a negative margin paid back as padding,
 * so the thumbnail and the name keep the section's gutter), with air between
 * the lines instead of hairlines, as Finder's and Notes' lists do. A hairline
 * under a rounded tile would read as the tile's own edge. The phone keeps the
 * grouped list's hairlines, which are its platform's.
 *
 * An image row draws the backend's small thumbnail of its file, never the
 * file, and asks for it only once the row is on screen: the file is read
 * whole only when the row is opened, so a list of pictures costs the webview
 * kilobytes whatever the files weigh.
 */
export default function Row({ attachment, onOpen, onSave, onRemove }: Props) {
  const { t } = useTranslation()
  const line = useRef<HTMLDivElement | null>(null)
  const thumbnail = useThumbnail(attachment, previewKind(attachment) === 'image', line)

  return (
    <div
      ref={line}
      data-testid="attachment-row"
      // No inset of its own: the list stands inside the section's
      // `EDGE_GUTTER`, with its heading and the verb under it.
      className={cx(
        'group flex items-center transition-colors hover:bg-hover',
        ROW_HAIRLINE,
        'md:-mx-2 md:rounded-xl md:px-2 md:inset-shadow-none md:hover:bg-hover/70'
      )}
    >
      <button
        type="button"
        data-testid="attachment-open-button"
        onClick={onOpen}
        className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 py-2.5 text-left max-md:active:bg-hover md:py-2"
      >
        <Thumb attachment={attachment} url={thumbnail} size="row" />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-base text-text max-md:text-md">
            {attachment.name}
          </span>
          <span className={`mt-0.5 block ${META}`}>{fileCaption(attachment)}</span>
        </span>
      </button>
      {!isMobile && (
        <span className={cx('flex flex-none items-center gap-0.5 pr-1', HOVER_ONLY)}>
          <IconButton muted title={t('Save…')} onClick={onSave} testid="row-save-button">
            <DownloadGlyph />
          </IconButton>
          <IconButton
            muted
            title={t('Remove attachment')}
            onClick={onRemove}
            testid="row-remove-button"
          >
            <TrashGlyph />
          </IconButton>
        </span>
      )}
    </div>
  )
}
