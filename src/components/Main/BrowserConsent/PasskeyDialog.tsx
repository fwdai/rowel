import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  ASSOCIATE_TIMEOUT_MS,
  browserPasskeyRespond,
  type PasskeyAccount,
  type PasskeyAsk
} from '@/api/browser'
import { closeConsentAsk } from '@/store'
import Frame from '@/components/elements/Frame'
import Button from '@/components/elements/Button'
import RadioList from '@/components/elements/RadioList'
import { useDates } from '@/hooks/useDates'

const TITLE_ID = 'browser-passkey-title'

/**
 * A page wants a passkey made, or used. The site named is the rpId — what the
 * passkey belongs to — and the page that asked is shown under it, so a
 * subdomain asking for its parent's passkey is in plain sight. A sign-in also
 * names the account it would be as, and offers the choice when the vault holds
 * several for the site: Rust signs with the one picked, and nothing else. As
 * with the associate dialog, the answer goes straight back to Rust, Escape and
 * the backdrop count as a refusal, and nothing is awaited; the dialog leaves on
 * Rust's own clock, and its answer names the ask it was drawn for, so one that
 * outlives its ask cannot hand a yes to the next ceremony's.
 */
export default function PasskeyDialog({ ask }: { ask: PasskeyAsk }) {
  const { t } = useTranslation()
  const { relativeLong } = useDates()
  // Which of a sign-in's accounts; the newest, first in the list, until the
  // user says otherwise. Mounted afresh for each ask (see `BrowserConsent`).
  const [account, setAccount] = useState(0)

  useEffect(() => {
    const expiry = setTimeout(closeConsentAsk, ASSOCIATE_TIMEOUT_MS)
    return () => clearTimeout(expiry)
  }, [ask.id])

  const accounts = ask.accounts ?? []
  const choice = accounts.length > 1

  const answer = (allow: boolean) => {
    closeConsentAsk()
    browserPasskeyRespond(ask.id, allow, choice ? account : undefined).catch(() => {})
  }
  const allow = () => answer(true)
  const deny = () => answer(false)

  const site = ask.rpId
  const nameOf = (a: PasskeyAccount) => a.userName || a.userDisplayName || t('Passkey')
  const title =
    ask.kind === 'get'
      ? accounts.length === 1
        ? t('{{site}} wants to sign in with your passkey for {{account}}', {
            site,
            account: nameOf(accounts[0])
          })
        : t('{{site}} wants to sign in with your passkey', { site })
      : ask.userName || ask.userDisplayName
        ? t('{{site}} wants to create a passkey for {{account}}', {
            site,
            account: ask.userName || ask.userDisplayName
          })
        : t('{{site}} wants to create a passkey', { site })

  return (
    <Frame
      onClose={deny}
      labelledBy={TITLE_ID}
      testid="browser-passkey-modal"
      fit="content"
      className="w-dialog"
    >
      <div className="p-7">
        <h2
          id={TITLE_ID}
          data-testid="browser-passkey-title"
          className="text-lg font-semibold tracking-display"
        >
          {title}
        </h2>
        <p className="mt-1.5 text-base text-text2">
          {t('Asked by {{origin}} through the browser extension.', { origin: ask.origin })}
        </p>

        {choice && (
          <div className="mt-4">
            <RadioList
              options={accounts.map((a, i) => ({
                value: String(i),
                label:
                  a.userDisplayName && a.userDisplayName !== nameOf(a)
                    ? `${nameOf(a)} (${a.userDisplayName})`
                    : nameOf(a),
                // When the passkey was added: a site can name two of them
                // alike, and this is what tells them apart.
                meta: a.createdAt ? relativeLong(a.createdAt) : undefined
              }))}
              value={String(account)}
              onChange={value => setAccount(Number(value))}
              name={t('Account')}
              testidPrefix="browser-passkey-account"
            />
          </div>
        )}

        <div className="mt-5 flex items-center gap-2">
          <Button size="md" onClick={allow} testid="browser-passkey-allow">
            {t('Allow')}
          </Button>
          <Button variant="pale" size="md" onClick={deny} testid="browser-passkey-deny">
            {t('Deny')}
          </Button>
        </div>
      </div>
    </Frame>
  )
}
