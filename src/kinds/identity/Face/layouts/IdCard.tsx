import Eye from '@/components/elements/Face/Eye'
import DocCell from '../DocCell'
import Holder from '../Holder'
import Plate from '../Plate'
import Silhouette from '../Silhouette'
import Status from '../Status'
import TypeLabel from '../TypeLabel'
import { placeOf } from '../place'
import type { LayoutProps } from '.'
import Masthead from './Masthead'

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
 * its plate, the holder beside the photograph with the photograph's ghost in
 * the stock behind, and along the foot the card's number and the personal
 * number a state knows its holder by.
 */
export default function IdCard({ docType, doc }: LayoutProps) {
  const place = placeOf(doc)
  const kind = (
    <span className="flex items-center gap-1.5">
      <TypeLabel docType={docType} />
      {CHIP}
    </span>
  )

  return (
    <>
      {/* The ghost image: the photograph a second time, small and faint, in the
          stock itself and under whatever is printed over it. */}
      <svg
        aria-hidden
        viewBox="0 0 64 60"
        className="pointer-events-none absolute left-[84%] top-[51%] -z-10 w-[12.8%] opacity-[0.07]"
        fill="currentColor"
      >
        <Silhouette />
      </svg>

      {/* The state is the card's title and what kind of card it is the line
          under it; a card with no state yet is titled by its kind alone. */}
      <Masthead lead={<Plate doc={doc} />} title={place || kind} sub={place && kind} />

      <Holder doc={doc}>
        <DocCell doc={doc} name="nationality" size={5} />
        <DocCell doc={doc} name="birth_date" size={11} />
        <DocCell doc={doc} name="sex" size={3} />
        <DocCell doc={doc} name="issue_date" size={11} />
        <DocCell doc={doc} name="expiry_date" size={11} />
        <DocCell doc={doc} name="authority" size={10} />
      </Holder>

      {/* The foot folds on a narrow card: the two numbers keep their line and
          the validity drops under them, rather than the card's number being
          squeezed to a column of single characters. */}
      <div className="mt-auto flex flex-none flex-wrap items-end justify-end gap-x-5 gap-y-2 pt-3">
        <DocCell doc={doc} name="number" wrap className="min-w-0 flex-1 basis-32" />
        <DocCell doc={doc} name="personal_number" size={12} className="flex-none" />
        <div className="flex flex-none items-end gap-2.5">
          <Status iso={doc.value('expiry_date')} />
          <Eye shown={doc.shown} onToggle={doc.toggle} testid="reveal-number" className="mb-0.5" />
        </div>
      </div>
    </>
  )
}
