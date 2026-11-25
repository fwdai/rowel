import Eye from '@/components/elements/Face/Eye'
import DocCell from '../DocCell'
import Holder from '../Holder'
import Term from '../Term'
import TypeLabel from '../TypeLabel'
import { placeOf } from '../place'
import type { LayoutProps } from '.'
import Masthead from './Masthead'

/**
 * A residence permit: the granting state's code set large in the corner beside
 * the card's title, the holder beside the photograph — a permit's holder is
 * another state's national, so the nationality leads — with the card's number
 * under them, and the term the permit runs for drawn along the foot.
 */
export default function Permit({ docType, doc }: LayoutProps) {
  return (
    <>
      <Masthead
        lead={
          <DocCell
            doc={doc}
            name="country"
            bare
            ink="text-xl font-semibold uppercase tracking-[0.06em]"
            size={4}
            className="flex-none"
          />
        }
        title={<TypeLabel docType={docType} />}
        sub={placeOf(doc)}
      />

      <Holder doc={doc}>
        <DocCell doc={doc} name="nationality" size={5} />
        <DocCell doc={doc} name="birth_date" size={11} />
        <DocCell doc={doc} name="authority" size={12} />
        <div className="flex w-full items-end gap-2.5">
          <DocCell doc={doc} name="number" wrap className="min-w-0 flex-1" />
          <Eye shown={doc.shown} onToggle={doc.toggle} testid="reveal-number" className="mb-0.5" />
        </div>
      </Holder>

      <div className="mt-auto flex-none pt-3">
        <Term doc={doc} />
      </div>
    </>
  )
}
