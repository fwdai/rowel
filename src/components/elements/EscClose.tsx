import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import { CloseGlyph } from '../Main/icons'
import Kbd from './Kbd'

interface Props {
  onClose: () => void
  disabled?: boolean
  /** The control's look; the layout and the hint are the same everywhere. */
  className?: string
  testid?: string
}

/**
 * The desktop's close control: the `esc` hint beside the glyph, at the right
 * end of whatever it closes. Says what closes it as well as closing it:
 * Escape does the same, so it is a pointer target and not the first tab stop
 * of every dialog. Every surface that can be closed on the desktop — a
 * dialog's header, the file preview's bar — draws this one, so the way out
 * is always in the same corner and spelled the same way.
 */
export default function EscClose({ onClose, disabled, className, testid = 'modal-close' }: Props) {
  const { t } = useTranslation()

  return (
    <button
      type="button"
      tabIndex={-1}
      aria-label={t('Close')}
      title={t('Close')}
      data-testid={testid}
      disabled={disabled}
      onClick={onClose}
      className={cx(
        'flex flex-none items-center gap-2 transition-colors',
        disabled ? 'cursor-default opacity-50' : 'cursor-pointer',
        className
      )}
    >
      <Kbd>esc</Kbd>
      <CloseGlyph />
    </button>
  )
}
