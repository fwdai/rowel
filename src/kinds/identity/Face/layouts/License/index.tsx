import Eye from '@/components/elements/Face/Eye'
import DocCell from '../../DocCell'
import Holder from '../../Holder'
import Plate from '../../Plate'
import Status from '../../Status'
import TypeLabel from '../../TypeLabel'
import { placeOf } from '../../place'
import type { LayoutProps } from '..'
import Masthead from '../Masthead'
import { INDEX } from './numbering'

/**
 * A driving licence, as the card is laid out the world over: the issuing
 * state's plate in the corner beside the card's title, the photograph, and the
 * fields under the numbers the licence standard gives them — 1 the name, 3 the
 * date of birth, 4a to 4c the issue, the expiry and the authority, 9 the
 * categories it covers, 5 the licence number.
 */
export default function License({ docType, doc }: LayoutProps) {
  return (
    <>
      <Masthead
        lead={<Plate doc={doc} />}
        title={<TypeLabel docType={docType} />}
        sub={placeOf(doc)}
      />

      <Holder doc={doc} index={INDEX.name} portrait="w-[24%]">
        <DocCell doc={doc} name="birth_date" index={INDEX.birth_date} size={11} />
        <DocCell doc={doc} name="issue_date" index={INDEX.issue_date} size={11} />
        <DocCell doc={doc} name="expiry_date" index={INDEX.expiry_date} size={11} />
        <DocCell doc={doc} name="authority" index={INDEX.authority} size={16} />
        <DocCell doc={doc} name="categories" index={INDEX.categories} size={16} />
      </Holder>

      <div className="mt-auto flex flex-none items-end gap-2.5 pt-3">
        <DocCell doc={doc} name="number" index={INDEX.number} wrap className="min-w-0 flex-1" />
        <Status iso={doc.value('expiry_date')} />
        <Eye shown={doc.shown} onToggle={doc.toggle} testid="reveal-number" className="mb-0.5" />
      </div>
    </>
  )
}
