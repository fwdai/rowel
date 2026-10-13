import type { ReactNode } from 'react'
import { cx } from '@/utils/cx'
import EscClose from './EscClose'
import { useFrameOwnsClose } from './frameContext'

/**
 * The header row every dialog shares: a title, a line saying what it is for,
 * and a close control that names its shortcut. A dialog using it passes
 * `hideClose` to its frame, so the card does not grow a second close button.
 * `disabled` greys the close out rather than hiding it, so the header does not
 * shift while an action is in flight.
 */
export default function DialogHeader({
  id,
  title,
  description,
  onClose,
  disabled,
  leading,
  className
}: {
  // id of the heading, for the frame's `aria-labelledby`.
  id: string
  title: ReactNode
  description?: ReactNode
  onClose: () => void
  disabled?: boolean
  // Sits before the title, e.g. a Back button.
  leading?: ReactNode
  // Horizontal padding override; defaults to `px-7`.
  className?: string
}) {
  // On a phone the frame's own bar has the close; `esc` means nothing there.
  const framed = useFrameOwnsClose()

  return (
    <div
      className={cx(
        'flex flex-none items-start gap-3 pb-3.5 inset-shadow-hairline',
        // Under a frame's own bar (a sheet's grabber row, a page's close bar)
        // the title sits 8px below it; a card has no bar, so it keeps 16px.
        framed ? 'pt-2' : 'pt-4',
        className ?? 'px-7'
      )}
    >
      {leading}
      <div className="min-w-0 flex-1">
        {/* h1: a modal dialog is its own document while it is open. */}
        <h1 id={id} className="truncate text-xl font-semibold tracking-display text-text">
          {title}
        </h1>
        {description != null && <p className="mt-0.5 text-sm text-text2">{description}</p>}
      </div>
      {!framed && (
        <EscClose
          onClose={onClose}
          disabled={disabled}
          className={cx(
            'h-7 rounded-sm px-1.5 text-text2',
            !disabled && 'hover:bg-hover hover:text-text'
          )}
        />
      )}
    </div>
  )
}
