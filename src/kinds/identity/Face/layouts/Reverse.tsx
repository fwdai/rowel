import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { DOC_TYPE_LABELS } from '../../templates'
import { placeOf } from '../place'
import type { LayoutProps } from '.'

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
        <div className="flex h-9 flex-none items-center truncate text-2xs uppercase tracking-[0.1em] text-(--face-ink2)">
          {t(DOC_TYPE_LABELS[docType])}
          {place && ` · ${place}`}
        </div>
        <div className="mt-auto flex min-w-0 items-end gap-4 pt-3">{children}</div>
      </div>
      {band}
    </>
  )
}
