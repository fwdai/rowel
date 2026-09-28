import { useUi } from '@/store'
import AssociateDialog from './AssociateDialog'

/**
 * The gate for a browser extension asking to connect. Keyed on the ask, so a
 * second one arriving while the first is up starts from a fresh name field
 * rather than carrying over what was typed for the other.
 */
export default function BrowserAssociate() {
  const ask = useUi(state => state.browserAsk)

  return ask ? <AssociateDialog key={ask.id} ask={ask} /> : null
}
