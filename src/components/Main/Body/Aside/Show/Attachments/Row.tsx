import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Attachment } from '@/api/types'
import { cx } from '@/utils/cx'
import { humanSize } from '@/utils/size'
import IconButton from '@/components/elements/IconButton'
import { STACK_RAIL } from '@/components/elements/fields/Row'
import { META, ROW_HAIRLINE } from '@/components/elements/tokens'
import {
  DownloadGlyph,
  FileArchiveGlyph,
  FileGlyph,
  FileImageGlyph,
  FileTextGlyph,
  TrashGlyph
} from '@/components/Main/icons'

interface Props {
  attachment: Attachment
  onSave: () => void
  onRemove: () => void
}

// The glyph says what kind of file it is, from the type the backend guessed
// off the name; anything it could not place is just a file.
const KindGlyph = ({ mime }: { mime?: string }) => {
  if (mime?.startsWith('image/')) return <FileImageGlyph />
  if (mime === 'application/zip') return <FileArchiveGlyph />
  if (mime?.startsWith('text/') || mime === 'application/pdf' || mime === 'application/json')
    return <FileTextGlyph />
  return <FileGlyph />
}

/**
 * One attached file: what it is, its name over its size, and the two things
 * that can be done with it. The name sits over the size the way a passkey's
 * site sits over its account, so the row needs no fold of its own — and its
 * rail takes the fields' touch sizing (`STACK_RAIL`), so under a finger in a
 * narrow container the buttons grow to the 44px target like every other row's.
 *
 * Remove is two presses on the same element, as the editor's Cancel is: the
 * first arms it ("Remove?"), the second removes; leaving it disarms.
 */
export default function AttachmentRow({ attachment, onSave, onRemove }: Props) {
  const { t } = useTranslation()
  const [armed, setArmed] = useState(false)

  return (
    <div
      data-testid="attachment-row"
      className={cx('flex items-center gap-3 px-3.5 py-3 max-md:px-4', ROW_HAIRLINE)}
    >
      <span className="flex-none text-text3">
        <KindGlyph mime={attachment.mime} />
      </span>
      <div className="min-w-0 flex-1">
        <div className="truncate text-base text-text" title={attachment.name}>
          {attachment.name}
        </div>
        <div className={`mt-0.5 ${META}`}>{humanSize(attachment.size)}</div>
      </div>
      <div className={cx('flex flex-none items-center gap-1', STACK_RAIL)}>
        <IconButton title={t('Save…')} onClick={onSave} testid="attachment-save-button">
          <DownloadGlyph />
        </IconButton>
        {armed ? (
          <button
            type="button"
            data-testid="attachment-remove-confirm"
            onClick={onRemove}
            onBlur={() => setArmed(false)}
            onMouseLeave={() => setArmed(false)}
            className="h-7 cursor-pointer rounded-sm px-2 text-base text-bad hover:brightness-110"
          >
            {t('Remove?')}
          </button>
        ) : (
          <IconButton
            title={t('Remove attachment')}
            onClick={() => setArmed(true)}
            testid="attachment-remove-button"
          >
            <TrashGlyph />
          </IconButton>
        )}
      </div>
    </div>
  )
}
