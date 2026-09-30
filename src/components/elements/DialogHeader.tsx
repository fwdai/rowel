import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import { CloseGlyph } from '../Main/icons'
import Kbd from './Kbd'
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
  const { t } = useTranslation()
  // On a phone the frame's own bar has the close; `esc` means nothing there.
  const framed = useFrameOwnsClose()

  return (
    <div
      className={cx(
        'flex flex-none items-start gap-3 pt-4 pb-3.5 inset-shadow-hairline',
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
      {/* Says what closes it as well as closing it: Escape does the same. */}
      {!framed && (
        <button
          type="button"
          aria-label={t('Close')}
          title={t('Close')}
          data-testid="modal-close"
          disabled={disabled}
          onClick={onClose}
          className={cx(
            'flex h-7 flex-none items-center gap-2 rounded-sm px-1.5 text-text2 transition-colors',
            disabled ? 'cursor-default opacity-50' : 'cursor-pointer hover:bg-hover hover:text-text'
          )}
        >
          <Kbd>esc</Kbd>
          <CloseGlyph />
        </button>
      )}
    </div>
  )
}
