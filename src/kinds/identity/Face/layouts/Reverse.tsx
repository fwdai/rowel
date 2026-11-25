import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import { DOC_TYPE_LABELS } from '../../templates'
import { placeOf } from '../place'
import type { Doc } from '../DocCell'
import type { LayoutProps } from '.'
import Masthead from './Masthead'
import type { Stock } from './stocks'

interface WatermarkProps {
  stock: Stock
  doc: Doc
  className?: string
}

/** The card's mark, large and faint in the stock of its back. */
export function Watermark({ stock, doc, className }: WatermarkProps) {
  return (
    <div aria-hidden className={cx('flex-none opacity-40', className)} style={{ color: stock.line }}>
      {stock.mark(88, doc)}
    </div>
  )
}

interface Props extends LayoutProps {
  /** What runs along the foot, edge to edge: the machine-readable zone, a barcode. */
  band: ReactNode
  /** What the back carries above that. */
  children: ReactNode
}

/**
 * The back of a card: a line saying whose back this is, what the card carries
 * there, and the band a machine reads along the foot. Nothing on it is typed —
 * the back is what the card says about itself, drawn from what the front holds.
 */
export default function Reverse({ docType, doc, band, children }: Props) {
  const { t } = useTranslation()
  const place = placeOf(doc)

  return (
    <>
      <div className="flex min-w-0 flex-1 flex-col px-5 pb-3 pt-4">
        <Masthead sub={`${t(DOC_TYPE_LABELS[docType])}${place && ` · ${place}`}`} />
        <div className="mt-auto flex min-w-0 items-end gap-4 pt-3">{children}</div>
      </div>
      {band}
    </>
  )
}
