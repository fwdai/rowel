import type { ReactNode } from 'react'
import { copy } from '@/services/copy'
import { cx } from '@/utils/cx'

interface Props {
  /** What pressing hands over — the real value, whatever is shown. */
  value: string
  /**
   * The accessible name. A masked value's text is twelve dots and names
   * nothing, so the row says what it is and what pressing does instead:
   * "Password · Copy".
   */
  label: string
  testid?: string
  /** The value's own face: size, ink, tracking, or a chip row's layout. */
  className?: string
  children: ReactNode
}

/**
 * A read row's value, as the copy control it is: a press hands the value over
 * and the toast says so. One element for every row that shows a value — the
 * typed fields, a custom pair, an env variable, a key's comment or
 * fingerprint, a scope list — so each is copied the same way. A finger cannot
 * hover a row to find a button, and text selection is off app-wide, so there
 * is nothing else a press on a value could mean; on the phone this is the
 * row's only copy control (see `ROW_COPY`).
 */
export default function CopyValue({ value, label, testid, className, children }: Props) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={() => copy(value)}
      data-testid={testid}
      className={cx('cursor-pointer text-left', className)}
    >
      {children}
    </button>
  )
}
