import { useRef } from 'react'
import { useTranslation } from 'react-i18next'
import { useDialogFocus } from '@/hooks/useDialogFocus'
import { useVisualViewport, viewportStyle } from '@/hooks/useVisualViewport'
import { ROW_HAIRLINE } from './tokens'

export interface SheetAction {
  testid: string
  label: string
  onSelect: () => void
}

interface Props {
  /** What is being decided, in the sheet's small grey heading. */
  title: string
  /** The choices, top to bottom. Cancel is added under them. */
  actions: SheetAction[]
  /** Backing out: the scrim, Cancel, or Escape. */
  onClose: () => void
  testid: string
}

// An action's row: the system's 57pt, centred, in the accent ink. The whole
// row is the target.
const ACTION =
  'flex h-14 w-full items-center justify-center px-4 text-center text-xl text-accent transition-colors active:bg-hover'

/**
 * The phone's choice among a few actions, drawn as the platform draws it: an
 * action sheet. Not the bottom sheet (`BottomSheet`), which is a surface with
 * content in it and a grabber to pull it down — this is a question with two or
 * three answers, and iOS gives that its own species: a floating group of
 * centred rows inset from the screen's edges, with Cancel as a separate pill
 * under them and the scrim to tap. No grabber, no close button, no chevrons:
 * the rows go to the system's own pickers, not deeper into the app.
 *
 * Used where the system's own sheet would say the wrong thing — attaching a
 * file, where the webview's file input would offer the camera too.
 *
 * As modal as every other frame: focus starts inside, Tab stays inside, Escape
 * closes, and it is sized from the visual viewport like `BottomSheet`.
 */
export default function ActionSheet({ title, actions, onClose, testid }: Props) {
  const { t } = useTranslation()
  const view = useVisualViewport()
  const frame = useRef<HTMLDivElement>(null)
  useDialogFocus(frame, onClose)
  const titleId = `${testid}-title`

  return (
    <div
      ref={frame}
      role="dialog"
      aria-modal="true"
      tabIndex={-1}
      aria-labelledby={titleId}
      data-testid={testid}
      data-frame="action-sheet"
      style={viewportStyle(view)}
      className="animate-fade fixed inset-0 z-50 flex flex-col justify-end"
    >
      <div className="absolute inset-0 bg-scrim" onClick={onClose} />
      {/* 8px off the sides and off the home indicator, as the system's sits. */}
      <div className="animate-rise relative mx-2 mb-[calc(env(safe-area-inset-bottom)+8px)] flex flex-col gap-2">
        <div className="overflow-hidden rounded-2xl bg-detail text-text">
          <div className={ROW_HAIRLINE}>
            <p id={titleId} className="px-4 pt-3.5 pb-3 text-center text-sm font-semibold text-text2">
              {title}
            </p>
          </div>
          {actions.map(action => (
            <div key={action.testid} className={ROW_HAIRLINE}>
              <button
                type="button"
                data-testid={action.testid}
                onClick={action.onSelect}
                className={ACTION}
              >
                {action.label}
              </button>
            </div>
          ))}
        </div>
        <button
          type="button"
          data-testid="action-sheet-cancel"
          onClick={onClose}
          className={`${ACTION} rounded-2xl bg-detail font-semibold`}
        >
          {t('Cancel')}
        </button>
      </div>
    </div>
  )
}
