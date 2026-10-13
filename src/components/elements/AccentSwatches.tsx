import { useTranslation } from 'react-i18next'
import { useRadioNav } from '@/hooks/useRadioNav'
import { cx } from '@/utils/cx'
import { ACCENTS, type Accent } from '@/theme'
import { CheckGlyph } from '@/components/Main/icons'

interface Props {
  value: Accent
  onChange: (next: Accent) => void
  /** The group's accessible name — the row label it sits beside. */
  name?: string
  testidPrefix?: string
}

/**
 * The accent picker: one disc per choice, in the colour it would set, the
 * chosen one ringed and ticked. Each disc carries its own `data-accent`, so
 * it is painted through the same tokens the app will be — a preview by
 * construction, not a second copy of the palette (see theme.css).
 *
 * A radiogroup like Segmented: one tab stop, arrows move the selection.
 */
export default function AccentSwatches({ value, onChange, name, testidPrefix }: Props) {
  const { t } = useTranslation()
  const { ref, onKeyDown } = useRadioNav(
    ACCENTS.map(accent => accent.id),
    value,
    onChange
  )

  return (
    <div
      ref={ref}
      role="radiogroup"
      aria-label={name}
      onKeyDown={onKeyDown}
      className="flex items-center gap-2 any-pointer-coarse:gap-3"
    >
      {ACCENTS.map(({ id, label }) => {
        const active = id === value
        return (
          <button
            key={id}
            type="button"
            role="radio"
            aria-checked={active}
            aria-label={t(label)}
            title={t(label)}
            tabIndex={active ? 0 : -1}
            data-accent={id}
            data-testid={testidPrefix && `${testidPrefix}-${id}`}
            onClick={() => onChange(id)}
            className={cx(
              'grid h-6 w-6 cursor-pointer place-items-center rounded-full bg-accent text-accent-fg transition-[transform,box-shadow] hover:scale-110 any-pointer-coarse:h-8 any-pointer-coarse:w-8',
              active && 'ring-2 ring-text ring-offset-2 ring-offset-detail'
            )}
          >
            {active && <CheckGlyph size={12} stroke={2.5} />}
          </button>
        )
      })}
    </div>
  )
}
