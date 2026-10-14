import { useTranslation } from 'react-i18next'
import type { Kind } from '@/kinds'
import { KIND_TINT } from '@/kinds/tint'
import Kbd from '@/components/elements/Kbd'
import { ROW_HAIRLINE } from '@/components/elements/tokens'
import { cx } from '@/utils/cx'

type Option = NonNullable<Kind['choices']>['options'][number]

interface Props {
  option: Option
  /** The kind's tint, under every answer's mark: they are all one kind. */
  tint: Kind['tint']
  /** The digit that picks it (see `index.tsx`), shown where there is a keyboard. */
  digit: number
  onSelect: () => void
}

/**
 * One answer to a kind's question, as a row of the picker's card: the
 * document's mark on the kind's tint, its name, and the digit that picks it.
 * The whole row is the target — 52px tall, past the 44pt a finger needs — and
 * it carries no chevron: it does not lead somewhere, it starts the entry.
 */
export default function ChoiceRow({ option, tint, digit, onSelect }: Props) {
  const { t } = useTranslation()
  const { Glyph } = option

  return (
    <div className={ROW_HAIRLINE}>
      <button
        type="button"
        data-testid={`add-choice-${option.value}`}
        onClick={onSelect}
        className="flex h-13 w-full cursor-pointer items-center gap-3.5 px-4 text-left transition-colors hover:bg-hover active:bg-hover"
      >
        <span
          className={cx('grid h-8 w-8 flex-none place-items-center rounded-sm', KIND_TINT[tint])}
        >
          <Glyph />
        </span>
        <span className="min-w-0 flex-1 truncate text-base font-medium text-text">
          {t(option.label)}
        </span>
        <span aria-hidden className="any-pointer-coarse:hidden">
          <Kbd>{digit}</Kbd>
        </span>
      </button>
    </div>
  )
}
