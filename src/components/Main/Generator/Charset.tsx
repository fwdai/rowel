import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import type { TKey } from '@/i18n'
import type { GeneratorSettings } from '@/services/generator'

type CharsetFlag = 'uppercase' | 'lowercase' | 'numbers' | 'symbols'

// The four classes a random password draws from, each named by a sample of
// itself. Order matches the pools in services/generator.
const CHARSETS: { flag: CharsetFlag; glyph: string; label: TKey }[] = [
  { flag: 'uppercase', glyph: 'A–Z', label: 'Uppercase' },
  { flag: 'lowercase', glyph: 'a–z', label: 'Lowercase' },
  { flag: 'numbers', glyph: '0–9', label: 'Numbers' },
  { flag: 'symbols', glyph: '!#$', label: 'Symbols' }
]

interface Props {
  settings: Pick<GeneratorSettings, CharsetFlag>
  onChange: (patch: Partial<GeneratorSettings>) => void
  // Glyph only, for a row with no room for words; the word stays as the name.
  compact?: boolean
  // md 28px beside settings rows, lg 36px in the dialog's control tier.
  size?: 'md' | 'lg'
  testidPrefix?: string
  className?: string
}

// THE character-class picker, shared by Settings › Security and the ⌘G dialog:
// one chip per class, pressed when the class is drawn from. The last class on
// stays on — a generator with nothing to draw from is not a setting — so that
// chip goes inert rather than the engine refusing later.
export default function CharsetChips({
  settings,
  onChange,
  compact,
  size = 'md',
  testidPrefix,
  className
}: Props) {
  const { t } = useTranslation()
  const on = CHARSETS.filter(({ flag }) => settings[flag]).length

  return (
    <div className={cx('flex flex-wrap gap-1.5', className)}>
      {CHARSETS.map(({ flag, glyph, label }) => {
        const active = settings[flag]
        const last = active && on === 1
        return (
          <button
            key={flag}
            type="button"
            aria-pressed={active}
            aria-disabled={last || undefined}
            aria-label={compact ? t(label) : undefined}
            title={compact ? t(label) : undefined}
            data-testid={testidPrefix && `${testidPrefix}-${flag}`}
            onClick={() => {
              if (!last) onChange({ [flag]: !active } as Partial<GeneratorSettings>)
            }}
            className={cx(
              'flex items-center gap-1.5 rounded-sm border px-2.5 text-base transition-colors any-pointer-coarse:h-11',
              size === 'lg' ? 'h-9' : 'h-7',
              active
                ? 'border-accent-line bg-accent-soft text-text'
                : 'border-line2 text-text2 hover:border-accent-line hover:text-text',
              last ? 'cursor-default' : 'cursor-pointer'
            )}
          >
            <span className="font-medium tabular-nums">{glyph}</span>
            {!compact && <span className={cx(active && 'text-text2')}>{t(label)}</span>}
          </button>
        )
      })}
    </div>
  )
}
