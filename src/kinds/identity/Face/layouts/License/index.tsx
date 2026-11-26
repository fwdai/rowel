import DocCell from '../../DocCell'
import Foot from '../../Foot'
import Holder from '../../Holder'
import type { LayoutProps } from '..'
import Masthead from '../Masthead'
import { INDEX } from './numbering'

/**
 * A driving licence, as the card is laid out the world over: the fields under
 * the numbers the licence standard gives them — 1 the name, 3 the date of
 * birth, 4a to 4c the issue, the expiry and the authority, 9 the categories it
 * covers, 5 the licence number.
 */
export default function License({ docType, doc }: LayoutProps) {
  return (
    <>
      <Masthead docType={docType} doc={doc} />
      <Holder doc={doc} index={INDEX.name}>
        <DocCell doc={doc} name="birth_date" index={INDEX.birth_date} size={11} />
        <DocCell doc={doc} name="issue_date" index={INDEX.issue_date} size={11} />
        <DocCell doc={doc} name="expiry_date" index={INDEX.expiry_date} size={11} />
        <DocCell doc={doc} name="authority" index={INDEX.authority} size={16} />
        <DocCell doc={doc} name="categories" index={INDEX.categories} size={16} />
      </Holder>
      <Foot doc={doc} index={INDEX.number} />
    </>
  )
}
