import Eye from '@/components/elements/Face/Eye'
import DocCell from '../DocCell'
import Holder from '../Holder'
import Plate from '../Plate'
import Status from '../Status'
import TypeLabel from '../TypeLabel'
import { placeOf } from '../place'
import type { LayoutProps } from '.'
import Masthead from './Masthead'

/**
 * A national identity card: the state that issued it across the head beside
 * its plate, the holder beside the photograph, and along the foot the card's
 * number and the personal number a state knows its holder by.
 */
export default function IdCard({ docType, doc }: LayoutProps) {
  const place = placeOf(doc)
  const kind = <TypeLabel docType={docType} />

  return (
    <>
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
