import { useTranslation } from 'react-i18next'
import type { ImportFormat } from '@/api/imports'
import BrandMark from '@/components/elements/BrandMark'
import { FEATURED } from './apps'
import Tile from './Tile'
import OtherTile from './OtherTile'

interface Props {
  active: string | null
  disabled: boolean
  onFormat: (format: ImportFormat) => void
  // On a device an app can hand its data to directly: such an app's tile says
  // how (`onExchange`) rather than picking an export file.
  exchange: boolean
  onExchange: () => void
}

// Six cards, three to a row: a tile per featured app, wearing the app's own
// mark where there is one and its initials otherwise, and a sixth that holds
// the rest behind a menu.
export default function Tiles({ active, disabled, onFormat, exchange, onExchange }: Props) {
  const { t } = useTranslation()
  return (
    <div className="grid grid-cols-1 gap-2.5 md:grid-cols-3">
      {FEATURED.map(app => {
        const exchangeHint = exchange ? app.exchangeHint : undefined
        return (
          <Tile
            key={app.key}
            badge={app.brand ? <BrandMark brand={app.brand} size={20} /> : app.badge}
            name={t(app.name)}
            hint={t(exchangeHint ?? app.hint)}
            active={active === app.key}
            disabled={disabled}
            onClick={() => (exchangeHint ? onExchange() : onFormat(app.format))}
            testid={`import-tile-${app.key}`}
          />
        )
      })}
      <OtherTile active={active} disabled={disabled} onFormat={onFormat} />
    </div>
  )
}
