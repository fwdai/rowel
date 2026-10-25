import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ASSOCIATE_TIMEOUT_MS, browserRespond, type AssociateAsk } from '@/api/browser'
import { closeConsentAsk } from '@/store'
import { keyFingerprint } from '@/lib/keyFingerprint'
import { platformLabel } from '@/lib/clientPlatform'
import Frame from '@/components/elements/Frame'
import Button from '@/components/elements/Button'
import Checkbox from '@/components/elements/Checkbox'
import Field from '@/components/elements/Field'
import { inputClass } from '@/components/elements/formStyles'

const TITLE_ID = 'browser-associate-title'
const NAME_ID = 'browser-associate-name'

/**
 * A client asking to be let in: a browser extension, or an agent. The copy
 * says which, because they are let in to very different things — an
 * extension fills logins, an agent only ever asks to have one used — and an
 * agent is named by the platform it said it runs on. The answer goes straight
 * back to Rust, which is holding the client's request open for it; Escape and
 * the backdrop count as a refusal. Nothing is awaited: Rust gives up on the
 * ask after a minute whether or not this answered, so a failed reply has no
 * one left to report to. The dialog leaves on the same clock, and its answer
 * names the ask it was drawn for, so one that somehow outlives its ask cannot
 * hand a yes to the next — not even the same client's retry.
 */
export default function AssociateDialog({ ask }: { ask: AssociateAsk }) {
  const publicKey = ask.key
  const { t } = useTranslation()
  const agent = ask.kind !== 'extension'
  const platform = platformLabel(ask.platform)
  const [name, setName] = useState(() => (agent ? (platform ?? t('Agent')) : t('Browser')))
  const [driven, setDriven] = useState(false)

  useEffect(() => {
    const expiry = setTimeout(closeConsentAsk, ASSOCIATE_TIMEOUT_MS)
    return () => clearTimeout(expiry)
  }, [ask.id])

  const answer = (reply: string | null) => {
    closeConsentAsk()
    browserRespond(ask.id, reply, !agent && reply !== null && driven).catch(() => {})
  }
  const allow = () => answer(name.trim() || (agent ? (platform ?? t('Agent')) : t('Browser')))
  const deny = () => answer(null)

  const title = agent
    ? platform
      ? t('An agent ({{platform}}) wants to connect to {{appName}}', { platform })
      : t('An agent wants to connect to {{appName}}')
    : t('A browser extension wants to connect to {{appName}}')
  const description = agent
    ? t(
        'Allow it only if you just set it up yourself. It will be able to ask to use your logins, each time with your say-so, and never to read them.'
      )
    : t(
        'Allow it only if you just connected KeePassXC-Browser yourself. It will be able to fill your logins.'
      )

  return (
    <Frame
      onClose={deny}
      labelledBy={TITLE_ID}
      testid="browser-associate-modal"
      fit="content"
      className="w-dialog"
    >
      <div className="p-7">
        <h2
          id={TITLE_ID}
          data-testid="browser-associate-title"
          className="text-lg font-semibold tracking-display"
        >
          {title}
        </h2>
        <p
          className="mt-1.5 text-base text-text2"
          data-testid={agent ? 'browser-associate-agent' : undefined}
        >
          {description}
        </p>

        <p className="mt-4 text-sm text-text2">
          {t('Key')}{' '}
          <span data-testid="browser-associate-fingerprint" className="font-mono text-text">
            {keyFingerprint(publicKey)}
          </span>
        </p>

        <div className="mt-4">
          <Field id={NAME_ID} label={t('Name')}>
            <input
              id={NAME_ID}
              value={name}
              data-testid="browser-associate-name"
              onChange={event => setName(event.target.value)}
              onKeyDown={event => event.key === 'Enter' && allow()}
              className={inputClass}
            />
          </Field>
        </div>

        {/* Rowel's mark, not the extension's: an agent driving the browser
            cannot turn it off from there. */}
        {!agent && (
          <div className="mt-4">
            <Checkbox checked={driven} onChange={setDriven} testid="browser-associate-driven">
              {t('Used by an AI agent')}
            </Checkbox>
            <p className="mt-1 pl-6.5 text-sm text-text2">
              {t('Every fill from it then waits for your approval in {{appName}}.')}
            </p>
          </div>
        )}

        <div className="mt-5 flex items-center gap-2">
          <Button size="md" onClick={allow} testid="browser-associate-allow">
            {t('Allow')}
          </Button>
          <Button variant="pale" size="md" onClick={deny} testid="browser-associate-deny">
            {t('Deny')}
          </Button>
        </div>
      </div>
    </Frame>
  )
}
