import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { countryName } from '@/utils/countries'
import { DOC_TYPE_LABELS } from '../../templates'
import Medallion from '../Medallion'
import Print from '../Print'
import { HEAD, SUBTITLE } from '../flow'
import type { LayoutProps, Stock } from '.'

interface Props extends LayoutProps {
  stock: Stock
  /** The card's mark, the same one its front wears. */
  mark: ReactNode
  /** What runs along the foot, edge to edge: the machine-readable zone, a barcode. */
  band: ReactNode
  /** What the back carries above that. */
  children: ReactNode
}

/**
 * The back of a card: the front's stock seen from behind, its mark in the same
 * corner to turn it over again, a line saying whose back this is, and the
 * band a machine reads along the foot. Nothing on it is typed — the back is
 * what the card says about itself, drawn from what the front holds.
 */
export default function Reverse({ docType, doc, flip, stock, mark, band, children }: Props) {
  const { t } = useTranslation()
  const country = doc.value('country')
  const place = countryName(country) ?? country

  return (
    <div className="flex min-w-0 flex-1 flex-col">
      <Print {...stock} mirrored />
      <Medallion line={stock.line} onFlip={flip}>
        {mark}
      </Medallion>

      <div className="flex min-w-0 flex-1 flex-col px-5 pb-3 pt-4">
        <header className={HEAD}>
          <div className={SUBTITLE}>
            {t(DOC_TYPE_LABELS[docType])}
            {place && ` · ${place}`}
          </div>
        </header>
        <div className="mt-auto flex min-w-0 items-end gap-4 pt-3">{children}</div>
      </div>

      {band}
    </div>
  )
}
