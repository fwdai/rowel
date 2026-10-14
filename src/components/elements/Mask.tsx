import { cx } from '@/utils/cx'
import AsteriskGlyph from './AsteriskGlyph'
import { MASK_LENGTH } from './tokens'

interface Props {
  /** Glyphs drawn. Fixed per use, so the mask says nothing about the secret's length. */
  count?: number
  className?: string
}

// What a masked secret is read as: a row of the brand asterisk — the mascot's
// body, a masked character — where a text field would draw bullets. Sized in
// em so it takes the line's type size, and filled with currentColor so it takes
// the line's ink. Hidden from assistive tech: the control around it (a
// CopyValue, the key strip) names what it is and what pressing does.
export default function Mask({ count = MASK_LENGTH, className }: Props) {
  return (
    <span
      data-testid="mask"
      aria-hidden
      className={cx('inline-flex items-center gap-[0.3em] align-middle', className)}
    >
      {Array.from({ length: count }, (_, i) => (
        <AsteriskGlyph key={i} size="0.6em" />
      ))}
    </span>
  )
}
