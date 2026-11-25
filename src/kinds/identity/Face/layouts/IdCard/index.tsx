import { useTranslation } from 'react-i18next'
import Eye from '@/components/elements/Face/Eye'
import { countryName } from '@/utils/countries'
import { DOC_TYPE_LABELS } from '../../../templates'
import DocCell from '../../DocCell'
import Hologram from '../../Hologram'
import Medallion from '../../Medallion'
import Plate from '../../Plate'
import Portrait from '../../Portrait'
import Print from '../../Print'
import Silhouette from '../../Silhouette'
import Status from '../../Status'
import { FIELDS, HEAD, NAME, SUBTITLE, TITLE } from '../../flow'
import type { LayoutProps } from '..'
import { STOCK } from './stock'

// The mark of a document with a chip in it, as ICAO draws it.
const CHIP = (
  <svg
    aria-hidden
    viewBox="0 0 20 12"
    className="h-2.5 w-auto flex-none"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.5"
  >
    <rect x="0.75" y="0.75" width="18.5" height="10.5" rx="1" />
    <path d="M0.75 6H6.6M13.4 6h5.85" />
    <circle cx="10" cy="6" r="3.4" />
  </svg>
)

/**
 * A national identity card: the state that issued it across the head beside
 * its plate, its emblem as the hologram in the corner, the holder beside the
 * photograph with the photograph's ghost in the stock behind, and along the
 * foot the card's number and the personal number a state knows its holder by.
 * On blue stock.
 */
export default function IdCard({ docType, doc, flip }: LayoutProps) {
  const { t } = useTranslation()
  const country = doc.value('country')
  const place = countryName(country) ?? country

  return (
    <div className="flex min-w-0 flex-1 flex-col px-5 pb-3.5 pt-4">
      <Print {...STOCK}>
        {/* The ghost image: the photograph a second time, small and faint, in the
            stock itself and under whatever is printed over it. */}
        <g transform="translate(386 148) scale(0.92)" fill="#1B1D21" opacity={0.07}>
          <Silhouette />
        </g>
      </Print>

      <Medallion line={STOCK.line} onFlip={flip}>
        <Hologram country={country} className="h-7 w-7" />
      </Medallion>

      <header className={HEAD}>
        <Plate doc={doc} />
        {/* The state is the card's title and what kind of card it is the line
            under it; a card with no state yet is titled by its kind alone. */}
        <div className="min-w-0 flex-1">
          {place && <div className={TITLE}>{place}</div>}
          <div className={`flex items-center gap-1.5 ${place ? SUBTITLE : TITLE}`}>
            <span data-testid="identity-doc-type" className="truncate">
              {t(DOC_TYPE_LABELS[docType])}
            </span>
            {CHIP}
          </div>
        </div>
      </header>

      <div className="mt-3.5 flex items-start gap-4">
        <Portrait />
        <div className={FIELDS}>
          <DocCell doc={doc} name="name" label="Holder" ink={NAME} wrap className="w-full" />
          <DocCell doc={doc} name="nationality" size={5} />
          <DocCell doc={doc} name="birth_date" size={11} />
          <DocCell doc={doc} name="sex" size={3} />
          <DocCell doc={doc} name="issue_date" size={11} />
          <DocCell doc={doc} name="expiry_date" size={11} />
          <DocCell doc={doc} name="authority" size={10} />
        </div>
      </div>

      {/* The foot folds on a narrow card: the two numbers keep their line and
          the validity drops under them, rather than the card's number being
          squeezed to a column of single characters. */}
      <div className="mt-auto flex flex-none flex-wrap items-end justify-end gap-x-5 gap-y-2 pt-3">
        <DocCell
          doc={doc}
          name="number"
          ink="text-xl tracking-secret"
          wrap
          className="min-w-0 flex-1 basis-32"
        />
        <DocCell doc={doc} name="personal_number" size={12} className="flex-none" />
        <div className="flex flex-none items-end gap-2.5">
          <Status iso={doc.value('expiry_date')} />
          <Eye shown={doc.shown} onToggle={doc.toggle} testid="reveal-number" className="mb-0.5" />
        </div>
      </div>
    </div>
  )
}
