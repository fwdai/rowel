import { useTranslation } from 'react-i18next'
import type { TKey } from '@/i18n'
import { useRadioNav } from '@/hooks/useRadioNav'
import { cx } from '@/utils/cx'
import {
  WORKSPACE_COLORS,
  WORKSPACE_COLOR_CLASSES,
  type WorkspaceColor
} from '@/lib/workspaceColor'
import { CheckGlyph } from '@/components/Main/icons'

const NAMES: Record<WorkspaceColor, TKey> = {
  indigo: 'Indigo',
  violet: 'Violet',
  green: 'Green',
  amber: 'Amber',
  rose: 'Rose',
  teal: 'Teal'
}

interface Props {
  // Nothing picked yet: a workspace that has only ever had its hashed hue.
  value: WorkspaceColor | null
  onChange: (color: WorkspaceColor) => void
  /** The group's accessible name — the field label it sits under. */
  label: string
  disabled?: boolean
  testidPrefix: string
}

// The tile's palette, one disc per colour. The picked one is ringed in its own
// colour, set off from the card, so the ring reads as that disc's and not as a
// focus outline. A radiogroup like AccentSwatches: one tab stop, arrows pick.
export default function ColorSwatches({ value, onChange, label, disabled, testidPrefix }: Props) {
  const { t } = useTranslation()
  const { ref, onKeyDown } = useRadioNav<WorkspaceColor | null>(
    [...WORKSPACE_COLORS],
    value,
    next => next && onChange(next)
  )

  return (
    <div
      ref={ref}
      role="radiogroup"
      aria-label={label}
      onKeyDown={onKeyDown}
      className="flex flex-wrap items-center gap-2.5 any-pointer-coarse:gap-3"
    >
      {WORKSPACE_COLORS.map((key, index) => {
        const active = key === value
        const { bg, ring } = WORKSPACE_COLOR_CLASSES[key]
        return (
          <button
            key={key}
            type="button"
            role="radio"
            aria-checked={active}
            aria-label={t(NAMES[key])}
            title={t(NAMES[key])}
            // With nothing picked the first disc holds the group's tab stop.
            tabIndex={active || (value === null && index === 0) ? 0 : -1}
            disabled={disabled}
            data-testid={`${testidPrefix}-color-${key}`}
            onClick={() => onChange(key)}
            className={cx(
              'grid h-7 w-7 cursor-pointer place-items-center rounded-full text-white transition-[transform,box-shadow] disabled:cursor-default any-pointer-coarse:h-8 any-pointer-coarse:w-8',
              bg,
              active
                ? cx('ring-2 ring-offset-2 ring-offset-card', ring)
                : 'shadow-[inset_0_0_0_1px_rgba(0,0,0,.12)] hover:scale-110 disabled:hover:scale-100'
            )}
          >
            {active && <CheckGlyph size={13} stroke={3} />}
          </button>
        )
      })}
    </div>
  )
}
