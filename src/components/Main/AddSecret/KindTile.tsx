import { useTranslation } from 'react-i18next'
import type { Kind } from '@/kinds'
import { KIND_TINT } from '@/kinds/tint'
import Kbd from '@/components/elements/Kbd'
import { cx } from '@/utils/cx'

interface Props {
  kind: Kind
  /** The digit that picks it (see `index.tsx`), shown where there is a keyboard. */
  digit: number
  onSelect: () => void
}

/**
 * One choice in the "Add a secret" grid: the kind's tinted glyph and its label.
 * A real button, so ⏎/Space activate it and the global :focus-visible ring is
 * all the focus styling it needs.
 *
 * Three to a row leaves room for a label only, so the line that says what the
 * kind holds rides on `title`, and a longer translation of the label ellipsizes
 * rather than reflowing the tile. `bg-card` is a gradient image, so the hover
 * has to drop it before its wash can show.
 */
export default function KindTile({ kind, digit, onSelect }: Props) {
  const { t } = useTranslation()
  const { Glyph } = kind

  return (
    <button
      type="button"
      data-testid={`add-kind-${kind.type}`}
      title={t(kind.description)}
      onClick={onSelect}
      className="relative flex min-w-0 cursor-pointer flex-col items-center gap-2 rounded-lg border border-line bg-card px-2 py-3.5 shadow-card transition-colors hover:border-line2 hover:bg-none hover:bg-hover"
    >
      <span aria-hidden className="absolute top-2 right-2 any-pointer-coarse:hidden">
        <Kbd>{digit}</Kbd>
      </span>
      <span
        className={cx(
          'grid h-10 w-10 flex-none place-items-center rounded-sm',
          KIND_TINT[kind.tint]
        )}
      >
        <Glyph size={18} />
      </span>
      <span className="max-w-full truncate text-base font-medium text-text">{t(kind.label)}</span>
    </button>
  )
}
