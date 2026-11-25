import { cx } from '@/utils/cx'
import type { DocType } from '../templates'
import type { Doc } from './DocCell'
import { mrz } from './mrz'

interface Props {
  docType: DocType
  doc: Doc
  /**
   * How the zone is set. On a passport's data page it is a strip of small type
   * along the foot. On the back of a card it is the card's lower third, in type
   * sized so its thirty columns run the card's width — which is what the zone
   * is on the real thing, and why a card needs a back to carry it.
   */
  card?: boolean
}

// Thirty columns across the card inside its 20px margins: a mono advance of
// 0.6em plus the tracking, with a little held back for a wider fallback font.
const CARD_TYPE = 'px-5 py-3 text-[calc((100cqw-2.5rem)/21.6)] leading-[1.45] tracking-[0.1em]'
const PAGE_TYPE = 'px-5 py-2 text-2xs leading-[1.6] tracking-[0.1em]'

/*
 * What a machine reads off the document: the MRZ, generated from the fields
 * themselves — check digits and all, see `mrz.ts` — and running the full width
 * of the paper the way it is printed. The one thing on a face set in the mono
 * tier, because it is the one thing on it that is machine text rather than
 * something a person reads. A document with no zone prints nothing.
 */
export default function MachineZone({ docType, doc, card }: Props) {
  const lines = mrz(docType, doc)
  if (!lines) return null

  return (
    <div className="@container border-t border-(--face-rule)">
      <div
        data-testid="identity-mrz"
        className={cx(
          'overflow-hidden font-mono',
          card ? `bg-white/70 ${CARD_TYPE}` : `bg-(--face-tail) ${PAGE_TYPE}`
        )}
      >
        {lines.map((line, row) => (
          <div key={row} className="whitespace-pre">
            {line}
          </div>
        ))}
      </div>
    </div>
  )
}
