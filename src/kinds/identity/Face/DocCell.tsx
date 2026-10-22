import { cx } from '@/utils/cx'
import type { TKey } from '@/i18n'
import { useDates } from '@/hooks/useDates'
import { specOf, type IdentityKey } from '../templates'
import Cell from '@/components/elements/Face/Cell'
import Mask from '@/components/elements/Mask'

/** What a face reads from: the document's values, and whether its secrets show. */
export interface Doc {
  /** A stored value, '' unless this document's template has a row for it. */
  value: (key: IdentityKey) => string
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
  className?: string
}

// One row of the template, printed on the face. The template's spec says what
// the value is — a date, a secret — and this turns that into how it shows, the
// same way the form's rows do for the editor: a secret is the brand mask every
// masked value line in the vault wears, fixed in length. How long the document
// has left is not a cell: it belongs to the document rather than to any one of
// its dates, so the face states it once, in the status beside the number.
export default function DocCell({ doc, name, bare, label, ink, wrap, className }: Props) {
  const { formatDate } = useDates()
  const spec = specOf(name)
  const value = doc.value(name)
  if (!value) return null

  const masked = spec.secret && !doc.shown

  return (
    <Cell
      name={name}
      label={bare ? undefined : (label ?? spec.label)}
      value={value}
      display={masked ? <Mask count={8} /> : spec.date ? formatDate(value) : value}
      ink={cx(ink ?? 'text-base leading-tight', masked && 'text-(--face-ink2)')}
      wrap={wrap && !masked}
      className={className}
    />
  )
}
