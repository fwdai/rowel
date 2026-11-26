import DocCell from '../DocCell'
import Foot from '../Foot'
import Holder from '../Holder'
import type { LayoutProps } from '.'
import Masthead from './Masthead'

/**
 * A national identity card: the state that issued it across the head, the
 * holder beside the photograph, and along the foot the card's number and the
 * personal number a state knows its holder by.
 */
export default function IdCard({ docType, doc }: LayoutProps) {
  return (
    <>
      <Masthead docType={docType} doc={doc} byState />
      <Holder doc={doc}>
        <DocCell doc={doc} name="nationality" size={5} />
        <DocCell doc={doc} name="birth_date" size={11} />
        <DocCell doc={doc} name="sex" size={3} />
        <DocCell doc={doc} name="issue_date" size={11} />
        <DocCell doc={doc} name="expiry_date" size={11} />
        <DocCell doc={doc} name="authority" size={10} />
      </Holder>
      <Foot doc={doc}>
        <DocCell doc={doc} name="personal_number" size={12} className="ml-2.5 flex-none" />
      </Foot>
    </>
  )
}
