import { useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Attachment } from '@/api/types'
import ConfirmSheet from '@/components/elements/ConfirmSheet'
import IconButton from '@/components/elements/IconButton'
import { META } from '@/components/elements/tokens'
import { useDialogFocus } from '@/hooks/useDialogFocus'
import { useVisualViewport, viewportStyle } from '@/hooks/useVisualViewport'
import { CloseGlyph, DownloadGlyph, TrashGlyph } from '@/components/Main/icons'
import { fileCaption, previewKind } from '../kind'
import { useAttachmentUrl, type AttachmentUrl } from '../useAttachmentUrl'
import Stage from './Stage'

const TITLE_ID = 'attachment-preview-title'
const REMOVE_TITLE_ID = 'remove-attachment-title'

// The bar's three controls: the phone's 44px header tier over a 20px glyph,
// in the standard ink — the menu and the star on the entry screen are this
// tier, and a lightbox on the desktop is big enough to share it. Not the
// `muted` tier, which is a dialog frame's own close receding behind a
// heading; these are the actions of the screen.
const BAR_ACTION = 'h-11 w-11'

interface Props {
  attachment: Attachment
  /** The file already read, by the row that drew its thumbnail. */
  given?: AttachmentUrl
  onSave: () => void
  onRemove: () => void
  onClose: () => void
}

/**
 * One file, open: the whole window, as Quick Look and Photos open one. A bar
 * with the way out on the left, the file's name and facts in the middle, and
 * on the right the two things that can be done with it — save it somewhere,
 * or remove it, which asks first, naming the file, so the finger that landed
 * on the wrong row finds out here. Under the bar, the file (`Stage`).
 *
 * One view for both shells. On a phone it is the screen, on the phone's
 * ground, since there is no other way to show a picture there; in a desktop
 * window the same view is a lightbox — the app dimmed and blurred behind it,
 * the file standing on that — which is how every app that lists files opens
 * one. The two are told apart by `max-md:`/`md:` on the same elements, never
 * by a prop. Not a `Frame`: neither the centred card nor the page sheet is
 * this shape, but it keeps their contract (a modal dialog that owns the
 * keyboard, sized to the visible viewport). Clicking the ground beside the
 * file closes it, as a lightbox does.
 */
export default function Preview({ attachment, given, onSave, onRemove, onClose }: Props) {
  const { t } = useTranslation()
  const frame = useRef<HTMLDivElement | null>(null)
  useDialogFocus(frame, onClose)
  const view = useVisualViewport()
  const [asking, setAsking] = useState(false)
  const kind = previewKind(attachment)
  const own = useAttachmentUrl(attachment, kind !== 'none' && !given)
  const file = given ?? own

  return (
    <div
      ref={frame}
      role="dialog"
      aria-modal="true"
      tabIndex={-1}
      aria-labelledby={TITLE_ID}
      data-testid="attachment-preview"
      style={viewportStyle(view)}
      className="animate-fade fixed inset-0 z-50 flex flex-col text-text max-md:bg-screen max-md:pt-[env(safe-area-inset-top)] max-md:pb-[env(safe-area-inset-bottom)] md:bg-scrim md:backdrop-blur-md"
    >
      <div className="flex h-14 flex-none items-center gap-1 px-2 max-md:inset-shadow-hairline">
        <IconButton
          label={t('Close')}
          onClick={onClose}
          testid="attachment-preview-close"
          className={BAR_ACTION}
        >
          <CloseGlyph size={20} />
        </IconButton>
        <div className="min-w-0 flex-1 text-center">
          <h1 id={TITLE_ID} className="truncate text-base font-medium">
            {attachment.name}
          </h1>
          <p className={META}>{fileCaption(attachment)}</p>
        </div>
        <IconButton
          title={t('Save…')}
          onClick={onSave}
          testid="preview-save-button"
          className={BAR_ACTION}
        >
          <DownloadGlyph size={20} />
        </IconButton>
        <IconButton
          title={t('Remove attachment')}
          onClick={() => setAsking(true)}
          testid="preview-remove-button"
          className={BAR_ACTION}
        >
          <TrashGlyph size={20} />
        </IconButton>
      </div>

      <Stage kind={kind} attachment={attachment} file={file} onSave={onSave} onGround={onClose} />

      {asking && (
        <ConfirmSheet
          id={REMOVE_TITLE_ID}
          testid="attachment-remove-sheet"
          actionTestid="attachment-remove-confirm"
          title={t('Remove “{{name}}”?', { name: attachment.name })}
          message={t('The file is removed from this entry. This can’t be undone.')}
          action={t('Remove attachment')}
          onConfirm={onRemove}
          onClose={() => setAsking(false)}
        />
      )}
    </div>
  )
}
