import type { ReactNode } from 'react'
import { useCopied } from '@/hooks/useCopied'
import { CheckGlyph } from '@/components/Main/icons'
import { cx } from '@/utils/cx'

interface Props {
  /** What pressing hands over — the real value, whatever is shown. */
  value: string
  /**
   * The accessible name. A masked value is a row of glyphs and names nothing,
   * so the row says what it is and what pressing does instead:
   * "Password · Copy".
   */
  label: string
  /** The row's label points here, so a click on it lands on the value. */
  id?: string
  testid?: string
  /** The value's own face: size, ink, tracking, or a chip row's layout. */
  className?: string
  children: ReactNode
}

/**
 * A read row's value, as the copy control it is: a press hands the value over
 * and a check takes the value's place for a beat to say so. One element for
 * every row that shows a value — the typed fields, a custom pair, an env
 * variable, a key's comment or fingerprint, a scope list, the one-time code —
 * so each is copied the same way. A finger cannot hover a row to find a
 * button, and text selection is off app-wide, so there is nothing else a press
 * on a value could mean; on the phone this is the row's only copy control (see
 * `ROW_COPY`).
 *
 * The value goes invisible rather than away, so the row keeps its size, and
 * the check is a glyph rather than words, so the button's text — what the
 * testid is read for — is still the value alone.
 */
export default function CopyValue({ value, label, id, testid, className, children }: Props) {
  const { copied, copy } = useCopied()
  return (
    <button
      id={id}
      type="button"
      aria-label={label}
      onClick={() => copy(value)}
      data-testid={testid}
      className={cx('relative cursor-pointer text-left', className)}
    >
      {copied ? (
        <>
          {/* `contents`, so a chip row's chips are still the button's flex items. */}
          <span className="invisible contents">{children}</span>
          <span
            data-testid="copied-mark"
            className="absolute inset-y-0 left-0 flex items-center text-good"
          >
            <CheckGlyph size={16} />
          </span>
        </>
      ) : (
        children
      )}
    </button>
  )
}
