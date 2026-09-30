import { useRef, useState, type PointerEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { useDialogFocus } from '@/hooks/useDialogFocus'
import { useForwardedRef } from '@/hooks/useForwardedRef'
import { useVisualViewport, viewportStyle } from '@/hooks/useVisualViewport'
import { cx } from '@/utils/cx'
import { CloseGlyph } from '../Main/icons'
import type { FrameProps } from './Frame'
import IconButton from './IconButton'

// How far the card has to be pulled down, or how fast it has to be flicked
// (px per ms, over at least FLICK_MIN_PX), before letting go dismisses it.
const DISMISS_PX = 80
const FLICK_SPEED = 0.5
const FLICK_MIN_PX = 16

/**
 * The compact frame for a dialog that is only as tall as what it says.
 *
 * A picker with six tiles in it has no business taking a whole screen: it
 * arrives from the bottom edge instead, over a scrim that dismisses it, with a
 * grabber saying which way it goes. It is as modal as the page sheet — focus
 * starts inside, Tab stays inside, Escape closes — and it is sized from the
 * visual viewport for the same reason `Sheet` is: if a field in it ever takes
 * focus, the keyboard must not land on top of it.
 *
 * A sheet whose content runs to the top edge leaves almost no scrim to tap and
 * a phone has no Escape, so it also has a close button and follows the grabber
 * down: pulled far or fast enough it closes, otherwise it springs back.
 */
export default function BottomSheet({ onClose, labelledBy, testid, ref, children }: FrameProps) {
  const { t } = useTranslation()
  const view = useVisualViewport()
  const [frame, setFrame] = useForwardedRef<HTMLDivElement>(ref)
  useDialogFocus(frame, onClose)
  // Where the drag started, and how far down the card is pulled while it lasts.
  const origin = useRef<{ y: number; at: number } | null>(null)
  const [pull, setPull] = useState<number | null>(null)

  const grab = (e: PointerEvent<HTMLDivElement>) => {
    // Capturing the pointer would take the close button's click away from it.
    if ((e.target as Element).closest('button')) return
    e.currentTarget.setPointerCapture(e.pointerId)
    origin.current = { y: e.clientY, at: e.timeStamp }
    setPull(0)
  }
  const drag = (e: PointerEvent<HTMLDivElement>) => {
    if (origin.current) setPull(Math.max(0, e.clientY - origin.current.y))
  }
  const release = (e: PointerEvent<HTMLDivElement>) => {
    const from = origin.current
    if (!from) return
    origin.current = null
    setPull(null)
    const dy = e.clientY - from.y
    const speed = dy / Math.max(1, e.timeStamp - from.at)
    if (dy > DISMISS_PX || (dy > FLICK_MIN_PX && speed > FLICK_SPEED)) onClose()
  }
  const cancel = () => {
    origin.current = null
    setPull(null)
  }

  return (
    <div
      ref={setFrame}
      role="dialog"
      aria-modal="true"
      tabIndex={-1}
      aria-labelledby={labelledBy}
      data-testid={testid}
      // Which frame won, for anything asking (tests, styling hooks) without
      // having to read class names off the element.
      data-frame="bottom-sheet"
      style={viewportStyle(view)}
      className="animate-fade fixed inset-0 z-50 flex flex-col justify-end"
    >
      {/* The scrim is its own layer rather than the frame's background: the card
          sits on top of it, so a tap that lands on one is never both. */}
      <div className="absolute inset-0 bg-scrim" onClick={onClose} />
      {/* `translate`, not `transform`: `rise` fills `transform` for good, and
          would win over an inline one. */}
      <div
        style={{ translate: `0 ${pull ?? 0}px` }}
        className={cx(
          'animate-rise relative flex max-h-full flex-col rounded-t-2xl border-t border-line2 bg-detail text-text shadow-float',
          pull === null && 'transition-[translate] duration-200 ease-out'
        )}
      >
        {/* Only this row drags: one started in the content below is a scroll. */}
        <div
          data-testid="bottom-sheet-grabber"
          onPointerDown={grab}
          onPointerMove={drag}
          onPointerUp={release}
          onPointerCancel={cancel}
          className="relative flex h-12 flex-none touch-none justify-center pt-2.5"
        >
          <span className="h-1 w-9 rounded-full bg-line2" />
          <IconButton
            muted
            testid="modal-close"
            label={t('Close')}
            onClick={onClose}
            className="absolute top-0.5 right-1 h-11 w-11"
          >
            <CloseGlyph size={18} />
          </IconButton>
        </div>
        {/* The home indicator lives under the card's last row; 24px keeps the
            content off it. */}
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain pb-[calc(env(safe-area-inset-bottom)+24px)]">
          {children}
        </div>
      </div>
    </div>
  )
}
