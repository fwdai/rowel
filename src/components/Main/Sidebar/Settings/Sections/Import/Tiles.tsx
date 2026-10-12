import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import type { ImportFormat } from '@/api/imports'
import { CARD, META, META_TYPE } from '@/components/elements/tokens'
import { FEATURED } from './apps'

interface Props {
  active: string | null
  disabled: boolean
  onFormat: (format: ImportFormat) => void
}

// A tile per featured app; the others are one dropdown row (OtherApps).
export default function Tiles({ active, disabled, onFormat }: Props) {
  const { t } = useTranslation()
  return (
    <div className="grid grid-cols-1 gap-2.5 md:grid-cols-3">
      {FEATURED.map(tile => (
        <button
          key={tile.key}
          type="button"
          disabled={disabled}
          data-testid={`import-tile-${tile.key}`}
          onClick={() => onFormat(tile.format)}
          className={cx(
            CARD,
            'flex cursor-pointer items-center gap-3 p-3 text-left transition-colors hover:border-accent-line',
            active === tile.key && 'border-accent-line',
            disabled && 'cursor-default opacity-50'
          )}
        >
          <div
            className={`grid h-[34px] w-[34px] flex-none place-items-center rounded-sm bg-tile ${META_TYPE} font-semibold text-text2`}
          >
            {tile.badge}
          </div>
          <div className="min-w-0">
            <div className="truncate text-base font-medium text-text">{t(tile.name)}</div>
            <div className={`mt-0.5 ${META}`}>{t(tile.hint)}</div>
          </div>
        </button>
      ))}
    </div>
  )
}
