import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  ASSOCIATE_TIMEOUT_MS,
  agentGrantRespond,
  type AgentGrantAsk,
  type GrantAnswer
} from '@/api/browser'
import { closeConsentAsk } from '@/store'
import Frame from '@/components/elements/Frame'
import ItemPicker from './ItemPicker'
import Choices from './Choices'
import StepUp from './StepUp'
import { clampDays } from './days'

const TITLE_ID = 'agent-grant-title'

/**
 * A client wants to use a login on a page: the extension in agent mode, or
 * an agent that wants the extension to log it in. The dialog names who asks
 * and where — and, for an agent, its own reason as text and the sign-out
 * that follows when it is done — offers the page's logins when the policy
 * admits more than one (the first picked), and takes one of three answers:
 * once, for N days — which writes a standing rule — or no. As with the other
 * two consent dialogs the answer goes straight back to Rust, which is holding
 * the client's request open for it; Escape, the backdrop and the window's
 * close count as a refusal; nothing is awaited, and the dialog leaves on
 * Rust's own clock. A step-up ask adds the verification Rust runs before the
 * yes counts: the biometric prompt, or the master password typed here and
 * sent with the answer.
 */
export default function GrantDialog({ ask }: { ask: AgentGrantAsk }) {
  const { t } = useTranslation()
  const [itemId, setItemId] = useState(ask.candidates[0]?.id ?? '')
  const [days, setDays] = useState(() => clampDays(ask.defaultStandingDays, 7))
  const [password, setPassword] = useState('')

  useEffect(() => {
    const expiry = setTimeout(closeConsentAsk, ASSOCIATE_TIMEOUT_MS)
    return () => clearTimeout(expiry)
  }, [ask.id])

  const needsPassword = ask.stepUp && ask.verify === 'password'
  const answer = (reply: GrantAnswer | null) => {
    closeConsentAsk()
    const secret = reply && needsPassword ? password : undefined
    agentGrantRespond(ask.id, reply, secret).catch(() => {})
  }
  const once = () => answer({ itemId, standingDays: null })
  const standing = () =>
    answer({ itemId, standingDays: clampDays(days, ask.defaultStandingDays) })
  const deny = () => answer(null)
  const picked = ask.candidates.find(candidate => candidate.id === itemId)
  const canAllow = !!picked && (!needsPassword || password.length > 0)
  const agent = ask.clientKind !== 'extension'
  const title = agent
    ? t('{{client}} wants to log into {{origin}}', { client: ask.clientName, origin: ask.origin })
    : t('{{client}} wants to use a login on {{origin}}', {
        client: ask.clientName,
        origin: ask.origin
      })

  return (
    <Frame
      onClose={deny}
      labelledBy={TITLE_ID}
      testid="agent-grant-dialog"
      fit="content"
      className="w-dialog"
    >
      <div className="p-7">
        <h2
          id={TITLE_ID}
          data-testid="agent-grant-title"
          className="text-lg font-semibold tracking-display"
        >
          {title}
        </h2>
        <p className="mt-1.5 text-base text-text2">
          {t('It would fill the login through the browser extension, as you would yourself.')}
        </p>
        {/* The agent's own words, shown as text and nothing more: a reason is
            for the user to weigh, never for the app to act on. */}
        {ask.reason && (
          <p className="mt-3 text-base text-text2" data-testid="agent-grant-reason">
            {t('Its reason: “{{reason}}”', { reason: ask.reason })}
          </p>
        )}
        {ask.endsSession && (
          <p className="mt-3 text-sm text-text2" data-testid="agent-grant-ends-session">
            {t('When this grant ends, {{appName}} will sign the browser that filled it out of {{origin}} and the rest of its site.', {
              origin: ask.origin
            })}
          </p>
        )}

        {ask.candidates.length > 1 ? (
          <ItemPicker candidates={ask.candidates} value={itemId} onChange={setItemId} />
        ) : (
          picked && (
            <p className="mt-4 text-base text-text" data-testid={`agent-grant-item-${picked.id}`}>
              {picked.title || t('Untitled')}
              {picked.username && <span className="text-text2"> · {picked.username}</span>}
            </p>
          )
        )}

        {ask.stepUp && (
          <StepUp verify={ask.verify ?? 'password'} password={password} onPassword={setPassword} />
        )}

        <Choices
          days={days}
          onDays={setDays}
          disabled={!canAllow}
          onOnce={once}
          onStanding={standing}
          onDeny={deny}
        />
      </div>
    </Frame>
  )
}
