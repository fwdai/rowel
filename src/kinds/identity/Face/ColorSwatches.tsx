import { useTranslation } from 'react-i18next'
import { useRadioNav } from '@/hooks/useRadioNav'
import { CheckGlyph } from '@/components/Main/icons'
import { CARD_COLORS, colorLabel, swatchOf, type CardColor } from './colors'

interface Props {
  /** The colour the card is in now — picked, or its type's own. */
  value: CardColor
  onChange: (color: CardColor) => void
}

/**
 * The card's colours, one disc each, under the card while it is being edited:
 * pressing one repaints the card above it, so the choice is made against the
 * thing it changes. A radiogroup like the app's other swatches (`AccentSwatches`,
 * the workspace's): one tab stop, arrows pick.
 */
export default function ColorSwatches({ value, onChange }: Props) {
  const { t } = useTranslation()
  const { ref, onKeyDown } = useRadioNav<CardColor>([...CARD_COLORS], value, onChange)

  return (
    <div
      ref={ref}
      role="radiogroup"
      aria-label={t('Card color')}
      onKeyDown={onKeyDown}
      className="mt-3 flex flex-wrap items-center gap-2.5 px-1 any-pointer-coarse:gap-3"
    >
      {CARD_COLORS.map(color => {
        const active = color === value
        return (
          <button
            key={color}
            type="button"
            role="radio"
            aria-checked={active}
            aria-label={t(colorLabel(color))}
            title={t(colorLabel(color))}
            tabIndex={active ? 0 : -1}
            data-testid={`card-color-${color}`}
            onClick={() => onChange(color)}
            style={{ background: swatchOf(color) }}
            className={
              active
                ? 'grid h-6 w-6 cursor-pointer place-items-center rounded-full text-white ring-2 ring-text2 ring-offset-2 ring-offset-detail any-pointer-coarse:h-8 any-pointer-coarse:w-8 max-md:ring-offset-screen'
                : 'grid h-6 w-6 cursor-pointer place-items-center rounded-full shadow-[inset_0_0_0_1px_rgba(0,0,0,.12)] transition-transform hover:scale-110 any-pointer-coarse:h-8 any-pointer-coarse:w-8'
            }
          >
            {active && <CheckGlyph size={12} stroke={2.5} />}
          </button>
        )
      })}
    </div>
  )
}
