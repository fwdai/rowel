import { useUi } from '@/store'
import AssociateDialog from './AssociateDialog'
import PasskeyDialog from './PasskeyDialog'

/**
 * The one gate for every security decision the browser extension puts to the
 * user: an extension asking to connect, or a page's passkey ceremony. One
 * dialog at most, whichever kind — Rust holds one ask at a time in one slot,
 * and the store one field — and a newer ask replaces the one on screen. Each
 * dialog is keyed by its ask, so nothing typed or picked for the last one
 * carries over to the next.
 */
export default function BrowserConsent() {
  const ask = useUi(state => state.consentAsk)

  if (!ask) return null
  return ask.kind === 'associate' ? (
    <AssociateDialog key={ask.ask.id} ask={ask.ask} />
  ) : (
    <PasskeyDialog key={ask.ask.id} ask={ask.ask} />
  )
}
