import { useEffect, useRef } from 'react'
import { useTranslation } from 'react-i18next'
import type { Attachment } from '@/api/types'
import IconButton from '@/components/elements/IconButton'
import { META } from '@/components/elements/tokens'
import { isTopmostDialog, useDialogFocus } from '@/hooks/useDialogFocus'
import { useVisualViewport, viewportStyle } from '@/hooks/useVisualViewport'
import { BackGlyph, CloseGlyph, DownloadGlyph, NextGlyph, TrashGlyph } from '@/components/Main/icons'
import { cx } from '@/utils/cx'
import { fileCaption, previewKind } from '../kind'
import { useAttachmentUrl } from '../useAttachmentUrl'
import Stage from './Stage'

const TITLE_ID = 'attachment-preview-title'

// The bar's three controls: the phone's 44px header tier over a 20px glyph,
// in the standard ink — the menu and the star on the entry screen are this
// tier, and a lightbox on the desktop is big enough to share it. Not the
// `muted` tier, which is a dialog frame's own close receding behind a
// heading; these are the actions of the screen.
const BAR_ACTION = 'h-11 w-11'

// The way to the neighbouring file, on the desktop: a round control floating
// at the stage's edge, as a lightbox's arrows do, standing clear of the file.
// The pair is the keyboard's arrows made visible — a phone has neither.
const STEP = 'absolute top-1/2 -translate-y-1/2 max-md:hidden'
const STEP_ACTION = `${BAR_ACTION} rounded-full bg-detail/80 shadow-float backdrop-blur-sm hover:bg-detail`

interface Props {
  attachment: Attachment
  /** Where the file stands in its list, for the counter and the arrows. */
  index: number
  count: number
  onStep: (by: -1 | 1) => void
  onSave: () => void
  /** Ask to remove the file; the section asks, over this view. */
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
 * The view is of one file but knows its list: the caption says "2 of 5", the
 * left and right arrow keys step to the neighbours, and on the desktop a
 * control at each edge of the stage does the same for the pointer, so a run
 * of scans is read through without going back to the list each time. The
 * ends do not wrap, as Quick Look's do not.
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
export default function Preview({
  attachment,
  index,
  count,
  onStep,
  onSave,
  onRemove,
  onClose
}: Props) {
  const { t } = useTranslation()
  const frame = useRef<HTMLDivElement | null>(null)
  useDialogFocus(frame, onClose)
  const view = useVisualViewport()
  const kind = previewKind(attachment)
  const file = useAttachmentUrl(attachment, kind !== 'none')
  const hasPrevious = index > 0
  const hasNext = index < count - 1

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      // The remove sheet over this view takes the keyboard while it is up.
      if (e.defaultPrevented || !isTopmostDialog(frame.current)) return
      if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return
      e.preventDefault()
      onStep(e.key === 'ArrowLeft' ? -1 : 1)
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onStep])

  const caption =
    count > 1
      ? `${fileCaption(attachment)} · ${t('{{index}} of {{count}}', { index: index + 1, count })}`
      : fileCaption(attachment)

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
          <p className={META} data-testid="preview-caption">
            {caption}
          </p>
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
          onClick={onRemove}
          testid="preview-remove-button"
          className={BAR_ACTION}
        >
          <TrashGlyph size={20} />
        </IconButton>
      </div>

      {/* Keyed by file, so stepping fades the next one in rather than
          swapping it under the eye; the bar above stays put. */}
      <div key={attachment.id} className="animate-fade relative flex min-h-0 flex-1 flex-col">
        <Stage kind={kind} attachment={attachment} file={file} onSave={onSave} onGround={onClose} />
        {count > 1 && (
          <>
            <div className={cx(STEP, 'left-4')}>
              <IconButton
                label={t('Previous file')}
                disabled={!hasPrevious}
                onClick={() => onStep(-1)}
                testid="preview-previous-button"
                className={STEP_ACTION}
              >
                <BackGlyph />
              </IconButton>
            </div>
            <div className={cx(STEP, 'right-4')}>
              <IconButton
                label={t('Next file')}
                disabled={!hasNext}
                onClick={() => onStep(1)}
                testid="preview-next-button"
                className={STEP_ACTION}
              >
                <NextGlyph />
              </IconButton>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
