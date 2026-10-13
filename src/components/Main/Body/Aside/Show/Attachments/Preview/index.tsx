import { useEffect, useRef } from 'react'
import { useTranslation } from 'react-i18next'
import type { Attachment } from '@/api/types'
import EscClose from '@/components/elements/EscClose'
import IconButton from '@/components/elements/IconButton'
import { BELOW_CHROME, META } from '@/components/elements/tokens'
import { isTopmostDialog, useDialogFocus } from '@/hooks/useDialogFocus'
import { useVisualViewport, viewportStyle } from '@/hooks/useVisualViewport'
import { BackGlyph, CloseGlyph, DownloadGlyph, NextGlyph, TrashGlyph } from '@/components/Main/icons'
import { cx } from '@/utils/cx'
import { fileCaption, previewKind } from '../kind'
import { useAttachmentUrl } from '../useAttachmentUrl'
import Stage from './Stage'

const TITLE_ID = 'attachment-preview-title'

// The phone bar's controls: the 44px header tier over a 20px glyph, in the
// standard ink — the menu and the star on the entry screen are this tier.
// Not the `muted` tier, which is a dialog frame's own close receding behind
// a heading; these are the actions of the screen.
const BAR_ACTION = 'h-11 w-11'

// A control standing on the desktop's dimmed, blurred ground: a glass tile —
// the tab bar's `bg-glass` with a hairline and a blur of its own — that turns
// white under the pointer. Bare glyphs on that ground read as watermarks, not
// as things to press; a tile gives each a surface and says it is a control.
const GLASS =
  'md:bg-glass md:text-text md:shadow-float md:ring-1 md:ring-line2 md:backdrop-blur-xl md:hover:bg-detail md:hover:text-text'
const TILE = `${GLASS} md:h-10 md:w-10 md:rounded-xl`
const PILL = `${GLASS} h-10 rounded-full px-3.5 text-text2`
// The way to the neighbouring file, floating at the stage's edge as a
// lightbox's arrows do, clear of the file. The pair is the keyboard's arrows
// made visible — a phone has neither.
const STEP = 'absolute top-1/2 -translate-y-1/2 max-md:hidden'
const STEP_ACTION = `${GLASS} ${BAR_ACTION} rounded-full`

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
 * with the file's name and facts in the middle and the things that can be
 * done with it at the ends — save it somewhere, remove it (which asks first,
 * naming the file, so the finger that landed on the wrong row finds out
 * here), and the way out. Under the bar, the file (`Stage`).
 *
 * Where the way out stands is the platform's: on the phone, Close is the
 * 44px control at the bar's left, where every pushed screen keeps its back;
 * on the desktop it is the `esc` close at the right end, where every dialog
 * of the app keeps it (`EscClose`), after the file's own actions. The bar
 * is a three-column grid so the name stays centred whatever stands at the
 * ends.
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
 *
 * The lightbox covers the window's content, not its chrome: it starts under
 * the wide shell's top bar (`BELOW_CHROME`), where the macOS traffic lights
 * are pinned and which stays the window's drag handle — the way Quick Look
 * keeps its own title bar over its toolbar. Covering it put this bar's
 * controls on top of the window's. The phone has no chrome bar, only the
 * safe area.
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
      className={cx(
        'animate-fade fixed inset-0 z-50 flex flex-col text-text max-md:bg-screen max-md:pt-[env(safe-area-inset-top)] max-md:pb-[env(safe-area-inset-bottom)] md:bg-scrim md:backdrop-blur-md',
        BELOW_CHROME
      )}
    >
      <div className="grid h-14 flex-none grid-cols-[1fr_minmax(0,3fr)_1fr] items-center gap-2 px-2 max-md:inset-shadow-hairline md:h-16 md:px-4">
        <div className="flex items-center">
          <IconButton
            label={t('Close')}
            onClick={onClose}
            testid="attachment-preview-close"
            className={cx(BAR_ACTION, 'md:hidden')}
          >
            <CloseGlyph size={20} />
          </IconButton>
        </div>
        <div className="min-w-0 text-center">
          <h1 id={TITLE_ID} className="truncate text-base font-medium">
            {attachment.name}
          </h1>
          <p className={META} data-testid="preview-caption">
            {caption}
          </p>
        </div>
        <div className="flex items-center justify-end gap-1 md:gap-2">
          <IconButton
            title={t('Save…')}
            onClick={onSave}
            testid="preview-save-button"
            className={cx(BAR_ACTION, TILE)}
          >
            <DownloadGlyph size={20} className="md:size-[18px]" />
          </IconButton>
          <IconButton
            title={t('Remove attachment')}
            onClick={onRemove}
            testid="preview-remove-button"
            className={cx(BAR_ACTION, TILE)}
          >
            <TrashGlyph size={20} className="md:size-[18px]" />
          </IconButton>
          {/* A breath between the file's actions and the way out. */}
          <EscClose
            onClose={onClose}
            testid="attachment-preview-esc"
            className={cx(PILL, 'ml-3 max-md:hidden')}
          />
        </div>
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
