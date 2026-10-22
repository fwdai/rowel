import { cx } from '@/utils/cx'
import type { TKey } from '@/i18n'
import { useDates } from '@/hooks/useDates'
import { specOf, type IdentityKey } from '../templates'
import Cell from '@/components/elements/Face/Cell'
import Mask from '@/components/elements/Mask'

/** What a face reads from and, editing, writes to. */
export interface Doc {
  /** A stored value, '' unless this document's template has a row for it. */
  value: (key: IdentityKey) => string
  /** Whether this document's template has the row at all. */
  has?: (key: IdentityKey) => boolean
  /** Writes one row of the draft; absent while reading. */
  set?: (key: IdentityKey, value: string) => void
  /** Whether the template needs the row filled before the document saves. */
  required?: (key: IdentityKey) => boolean
  shown: boolean
  toggle: () => void
}

interface Props {
  doc: Doc
  name: IdentityKey
  /** Drop the caption: the header prints the country code on its own. */
  bare?: boolean
  /** A caption other than the field's own — the name reads as "Holder" here. */
  label?: TKey
  ink?: string
  wrap?: boolean
  /** The box's width in characters while editing; without it the cell's own. */
  size?: number
  className?: string
}

// One row of the template, printed on the face. The template's spec says what
// the value is — a date, a secret — and this turns that into how it shows and
// how it is typed: a date in the user's pattern, stored as ISO on blur; a
// secret under the brand mask every masked value line in the vault wears,
// fixed in length, and typed into dots once the eye is shut. How long the
// document has left is not a cell: it belongs to the document rather than to
// any one of its dates, so the face states it once, in the status beside the
// number.
export default function DocCell({ doc, name, bare, label, ink, wrap, size, className }: Props) {
  const { format, formatDate, toIsoDate } = useDates()
  const spec = specOf(name)
  const value = doc.value(name)
  const editing = !!doc.set
  // A row the template does not have is never printed. Reading, an empty cell
  // prints nothing either; editing, every cell of the template is a box to fill.
  if (doc.has && !doc.has(name)) return null
  if (!value && !editing) return null

  const masked = !!spec.secret && !doc.shown

  return (
    <Cell
      name={name}
      label={label ?? spec.label}
      caption={bare ? '' : undefined}
      value={value}
      display={masked ? <Mask count={8} /> : spec.date ? formatDate(value) : value}
      box={spec.date ? formatDate(value) : undefined}
      onChange={doc.set ? typed => doc.set?.(name, typed) : undefined}
      normalize={spec.date ? toIsoDate : undefined}
      placeholder={spec.date ? format : spec.placeholder}
      maxLength={spec.date ? 10 : spec.maxLength}
      size={size}
      required={doc.required?.(name)}
      masked={masked}
      ink={cx(ink ?? 'text-base leading-tight', masked && !editing && 'text-(--face-ink2)')}
      wrap={wrap && !masked}
      className={className}
    />
  )
}
