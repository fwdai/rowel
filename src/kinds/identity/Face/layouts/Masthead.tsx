import type { DocType } from '../../templates'
import type { Doc } from '../DocCell'
import Plate from '../Plate'
import TypeLabel from '../TypeLabel'
import { placeOf } from '../place'

interface Props {
  docType: DocType
  doc: Doc
  /**
   * Title the card by the state that issued it, with what kind of card it is
   * the line under — as a national ID card reads. A card with no state yet is
   * titled by its kind either way.
   */
  byState?: boolean
}

const TITLE = 'truncate text-base font-semibold uppercase leading-tight tracking-label'
const SUB = 'truncate text-2xs uppercase tracking-[0.1em] text-(--face-ink2)'

/**
 * A card's masthead: the issuing state's plate, what the card is and whose,
 * across its head — cleared on the right for the mark in the corner (see
 * `Card`).
 */
export default function Masthead({ docType, doc, byState }: Props) {
  const place = placeOf(doc)
  const kind = <TypeLabel docType={docType} />
  const [title, sub] = byState && place ? [place, kind] : [kind, place]

  return (
    <header className="flex h-9 flex-none items-center gap-3 pr-12">
      <Plate doc={doc} />
      <div className="min-w-0 flex-1">
        <div className={TITLE}>{title}</div>
        {sub && <div className={SUB}>{sub}</div>}
      </div>
    </header>
  )
}
