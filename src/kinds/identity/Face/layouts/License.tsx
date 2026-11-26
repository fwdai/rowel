import DocCell from '../DocCell'
import Foot from '../Foot'
import Holder from '../Holder'
import type { LayoutProps } from '.'
import Masthead from './Masthead'

/**
 * A driving licence, as the card is laid out the world over: the fields under
 * the numbers the licence standard (ISO/IEC 18013-1) gives them, the same on a
 * card from any country — 1 the name, 3 the date of birth, 4a to 4c the issue,
 * the expiry and the authority, 9 the categories it covers, 5 the licence
 * number.
 */
export default function License({ docType, doc }: LayoutProps) {
  return (
    <>
      <Masthead docType={docType} doc={doc} />
      <Holder doc={doc} index="1.">
        <DocCell doc={doc} name="birth_date" index="3." size={11} />
        <DocCell doc={doc} name="issue_date" index="4a." size={11} />
        <DocCell doc={doc} name="expiry_date" index="4b." size={11} />
        <DocCell doc={doc} name="authority" index="4c." size={16} />
        <DocCell doc={doc} name="categories" index="9." size={16} />
      </Holder>
      <Foot doc={doc} index="5." />
    </>
  )
}
