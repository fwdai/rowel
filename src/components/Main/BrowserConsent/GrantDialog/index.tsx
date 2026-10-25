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
import Target from './Target'
import { targetText } from './targetText'
import { clampDays } from './days'

const TITLE_ID = 'agent-grant-title'

/**
 * A client wants to use an item somewhere: the extension in agent mode or an
 * agent that wants the extension to log it in (a fill), or an agent that
 * wants a secret sent in requests to an API (a header) or given to one
 * command (env). The dialog names who asks and where — and, for an agent,
 * its own reason as text and, for a fill, the sign-out that follows when it
 * is done — offers the items when the policy admits more than one (the first
 * picked), and takes one of three answers: once, for N days — which writes a
 * standing rule — or no. As with the other two consent dialogs the answer
 * goes straight back to Rust, which is holding the client's request open for
 * it; Escape, the backdrop and the window's close count as a refusal; nothing
 * is awaited, and the dialog leaves on Rust's own clock. A step-up ask adds
 * the verification Rust runs before the yes counts: the biometric prompt, or
 * the master password typed here and sent with the answer.
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
  const secret = ask.mode === 'header' || ask.mode === 'env'
  const target = targetText(ask.targets[0])
  const title =
    ask.mode === 'env'
      ? t('{{client}} wants to give a secret to a command', { client: ask.clientName })
      : ask.mode === 'header'
        ? t('{{client}} wants to send a secret in requests to {{target}}', {
            client: ask.clientName,
            target
          })
        : ask.clientKind !== 'extension'
          ? t('{{client}} wants to log into {{origin}}', {
              client: ask.clientName,
              origin: target
            })
          : t('{{client}} wants to use a login on {{origin}}', {
              client: ask.clientName,
              origin: target
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
        <Target ask={ask} />
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
              origin: target
            })}
          </p>
        )}

        {/* A secret request is covered whole: every item it named goes to
            the command or the request, so they are listed, not picked, and
            Rust ignores which id the answer carries. A site request offers
            the page's logins for one to be picked. */}
        {secret || ask.candidates.length === 1 ? (
          ask.candidates.map(candidate => (
            <p
              key={candidate.id}
              className="mt-4 text-base text-text"
              data-testid={`agent-grant-item-${candidate.id}`}
            >
              {candidate.title || t('Untitled')}
              {candidate.username && <span className="text-text2"> · {candidate.username}</span>}
            </p>
          ))
        ) : (
          <ItemPicker candidates={ask.candidates} value={itemId} onChange={setItemId} />
        )}

        {ask.stepUp && (
          <StepUp verify={ask.verify ?? 'password'} password={password} onPassword={setPassword} />
        )}

        <Choices
          mode={ask.mode}
          several={ask.candidates.length > 1}
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
