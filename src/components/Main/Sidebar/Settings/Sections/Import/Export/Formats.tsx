import { useTranslation } from 'react-i18next'
import type { ExportFormat } from '@/api/imports'
import type { TKey } from '@/i18n'
import { useRadioNav } from '@/hooks/useRadioNav'
import { cx } from '@/utils/cx'
import { META, PICK_CARD, PICKED, UNPICKED } from '@/components/elements/tokens'

const FORMATS: { value: ExportFormat; name: TKey; extension: string; hint: TKey }[] = [
  {
    value: 'bitwarden',
    name: 'Bitwarden',
    extension: '.json',
    hint: 'Also opens in Proton Pass and Vaultwarden'
  },
  {
    value: 'cxf',
    name: 'FIDO CXF',
    extension: '.json',
    hint: 'Credential Exchange Format, the cross-app standard'
  },
  {
    value: 'csv',
    name: 'Generic CSV',
    extension: '.csv',
    hint: 'Spreadsheets and most managers. Logins only.'
  }
]

const VALUES = FORMATS.map(format => format.value)

interface Props {
  value: ExportFormat
  onChange: (value: ExportFormat) => void
  label: string
}

// The export formats as cards, one of which is always chosen: a radio group,
// with the arrow keys moving the choice as Segmented's do.
export default function Formats({ value, onChange, label }: Props) {
  const { t } = useTranslation()
  const { ref, onKeyDown } = useRadioNav(VALUES, value, onChange)

  return (
    <div
      ref={ref}
      role="radiogroup"
      aria-label={label}
      onKeyDown={onKeyDown}
      className="grid grid-cols-1 gap-2.5 md:grid-cols-3"
    >
      {FORMATS.map(format => {
        const active = format.value === value
        return (
          <button
            key={format.value}
            type="button"
            role="radio"
            aria-checked={active}
            tabIndex={active ? 0 : -1}
            data-testid={`settings-export-format-${format.value}`}
            onClick={() => onChange(format.value)}
            className={cx(PICK_CARD, 'p-4', active ? PICKED : UNPICKED)}
          >
            <div className="flex items-center gap-2">
              <span
                aria-hidden
                className={cx(
                  'grid h-4 w-4 flex-none place-items-center rounded-full border',
                  active ? 'border-accent' : 'border-line2'
                )}
              >
                {active && <span className="h-2 w-2 rounded-full bg-accent" />}
              </span>
              <span className="min-w-0 flex-1 truncate text-base font-medium text-text">
                {t(format.name)}
              </span>
              <span className={META}>{format.extension}</span>
            </div>
            <div className="mt-1.5 text-sm text-text2">{t(format.hint)}</div>
          </button>
        )
      })}
    </div>
  )
}
