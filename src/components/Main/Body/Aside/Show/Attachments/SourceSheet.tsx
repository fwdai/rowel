import { useTranslation } from 'react-i18next'
import ActionRow from '@/components/elements/ActionRow'
import DialogHeader from '@/components/elements/DialogHeader'
import Frame from '@/components/elements/Frame'
import { CARD } from '@/components/elements/tokens'
import { FolderGlyph, ImageGlyph } from '@/components/Main/icons'
import type { AttachSource } from './useAttachments'

const TITLE_ID = 'attach-source-title'

interface Props {
  onPick: (from: AttachSource) => void
  onClose: () => void
}

/**
 * Where the file comes from, asked the way a phone asks it: an action sheet
 * from the bottom edge, one tappable row per source, the scrim or the grabber
 * to back out. The two rows are the two pickers iOS keeps apart — the photo
 * library and the file browser — under the names the system itself uses for
 * them, so the choice reads like every other app's attach sheet.
 *
 * `fit="content"`: two rows are little enough to answer from the bottom edge.
 * Only the phone shell mounts this (see `AttachButton`); desktop goes straight
 * to a file dialog whose own sidebar reaches the library.
 */
export default function SourceSheet({ onPick, onClose }: Props) {
  const { t } = useTranslation()

  return (
    <Frame
      onClose={onClose}
      labelledBy={TITLE_ID}
      testid="attach-source-sheet"
      fit="content"
      hideClose
    >
      <DialogHeader id={TITLE_ID} title={t('Attach file')} onClose={onClose} className="px-5" />
      <div className="px-5 pt-1 pb-5">
        <div className={CARD}>
          <ActionRow
            testid="attach-photo-button"
            glyph={<ImageGlyph size={16} />}
            label={t('Photo Library')}
            onClick={() => onPick('photo')}
          />
          <ActionRow
            testid="attach-from-files-button"
            glyph={<FolderGlyph size={16} />}
            label={t('Choose File…')}
            onClick={() => onPick('attachment')}
          />
        </div>
      </div>
    </Frame>
  )
}
