import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ASSOCIATE_TIMEOUT_MS, browserRespond, type AssociateAsk } from '@/api/browser'
import { closeConsentAsk } from '@/store'
import { keyFingerprint } from '@/lib/keyFingerprint'
import Frame from '@/components/elements/Frame'
import Button from '@/components/elements/Button'
import Field from '@/components/elements/Field'
import { inputClass } from '@/components/elements/formStyles'

const TITLE_ID = 'browser-associate-title'
const NAME_ID = 'browser-associate-name'

/**
 * An extension asking to be let in. The answer goes straight back to Rust,
 * which is holding the extension's request open for it; Escape and the
 * backdrop count as a refusal. Nothing is awaited: Rust gives up on the ask
 * after a minute whether or not this answered, so a failed reply has no one
 * left to report to. The dialog leaves on the same clock, and its answer
 * names the ask it was drawn for, so one that somehow outlives its ask
 * cannot hand a yes to the next — not even the same extension's retry.
 */
export default function AssociateDialog({ ask }: { ask: AssociateAsk }) {
  const publicKey = ask.key
  const { t } = useTranslation()
  const [name, setName] = useState(() => t('Browser'))

  useEffect(() => {
    const expiry = setTimeout(closeConsentAsk, ASSOCIATE_TIMEOUT_MS)
    return () => clearTimeout(expiry)
  }, [ask.id])

  const answer = (reply: string | null) => {
    closeConsentAsk()
    browserRespond(ask.id, reply).catch(() => {})
  }
  const allow = () => answer(name.trim() || t('Browser'))
  const deny = () => answer(null)

  return (
    <Frame
      onClose={deny}
      labelledBy={TITLE_ID}
      testid="browser-associate-modal"
      fit="content"
      className="w-dialog"
    >
      <div className="p-7">
        <h2 id={TITLE_ID} className="text-lg font-semibold tracking-display">
          {t('A browser extension wants to connect to {{appName}}')}
        </h2>
        <p className="mt-1.5 text-base text-text2">
          {t(
            'Allow it only if you just connected KeePassXC-Browser yourself. It will be able to fill your logins.'
          )}
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
