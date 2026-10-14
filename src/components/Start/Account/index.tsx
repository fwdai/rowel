import { useState, type ChangeEvent } from 'react'
import { useTranslation } from 'react-i18next'
import Masterpass from '@/components/elements/Masterpass'
import Button from '@/components/elements/Button'
import AccountVaults from '@/components/elements/AccountVaults'
import { isMobile } from '@/lib/platform'
import type { UnlockResult } from '@/api/types'
import { setupRestoreFromAccount } from '@/api/setup'
import { providerName } from '@/api/provider'
import { waitingFor } from '@/components/elements/providerCopy'
import {
  useApp,
  selectedRemotePack,
  setupAccountSelect,
  connectAccount,
  switchSetupAccount
} from '@/store'
import StepHeader from '../shared/StepHeader'
import SpinnerCard from '../shared/SpinnerCard'
import TextLink from '../shared/TextLink'
import { COLUMN, ACTIONS, FOOTNOTE } from '../shared/layout'
import { unsealError } from '../shared/errors'

interface Props {
  /** Nothing in this account: go and make some, with sync already wired up. */
  onStartFresh: () => void
  /** Desktop's other route in, for an account that is not where the data is. */
  onUseFile: () => void
  onRestored: (result: UnlockResult) => Promise<void>
}

// Restoring from the account the welcome's press connected. Everything on
// screen follows the probe in the store — pending while consent is out with the
// browser, then one of found, empty or failed — so a result that arrives while
// the user is reading simply redraws the screen.
export default function Account({ onStartFresh, onUseFile, onRestored }: Props) {
  const { t } = useTranslation()
  const account = useApp(state => state.setupAccount)
  const file = useApp(selectedRemotePack)
  const provider = account.provider ?? 'googleDrive'
  const name = { provider: providerName(provider) }
  const eyebrow = t('Restore · {{provider}}', name)
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const change = (event: ChangeEvent<HTMLInputElement>) => {
    setError(null)
    setPassword(event.currentTarget.value)
  }

  const unlock = () => {
    if (busy || !password || !file) return
    setBusy(true)
    setError(null)
    setupRestoreFromAccount(password, file.id)
      .then(onRestored)
      .catch((err: unknown) => {
        setBusy(false)
        setError(
          unsealError(
            t,
            err,
            t("That isn't the password this was sealed with. Try the one you use on your other devices.")
          )
        )
      })
  }

  // Withdrawn while the unlock runs: the restore has already taken the account
  // it is restoring from, and the backend would refuse a switch anyway — so the
  // offer is not made, rather than made and then declined.
  const links = busy ? null : (
    <div className={`${FOOTNOTE} flex items-center justify-center gap-4`}>
      <TextLink testid="drive-switch-account" onClick={switchSetupAccount}>
        {t('Switch account')}
      </TextLink>
      {!isMobile && (
        <TextLink testid="drive-use-file" onClick={onUseFile}>
          {t('Use a backup file instead')}
        </TextLink>
      )}
    </div>
  )

  if (account.status === 'error')
    return (
      <>
        <StepHeader
          eyebrow={eyebrow}
          tone="bad"
          title={t('{{provider}} did not answer', name)}
          body={account.error || t('Something went wrong')}
        />
        <div className={ACTIONS}>
          <Button block testid="drive-retry-button" onClick={() => connectAccount(provider)}>
            {t('Try again')}
          </Button>
        </div>
        {links}
      </>
    )

  if (account.status === 'empty')
    return (
      <>
        <StepHeader
          eyebrow={eyebrow}
          title={t('Nothing here yet')}
          body={t('This {{provider}} account has no Rowel data. Start fresh and it will sync here.', name)}
        />
        <div className={ACTIONS}>
          <Button block testid="drive-start-fresh-button" onClick={onStartFresh}>
            {t('Start fresh')}
          </Button>
        </div>
        {links}
      </>
    )

  if (account.status !== 'found')
    return (
      <>
        <StepHeader eyebrow={eyebrow} busy title={t('Looking for your data…')} />
        <div className={`${COLUMN} mt-9`}>
          <SpinnerCard testid="drive-spinner" caption={waitingFor(t, provider)} />
        </div>
      </>
    )

  // What the account holds decides the wording: a choice to make, or one vault
  // to unlock. `AccountVaults` draws whichever of the two it is.
  const several = account.files.length > 1

  return (
    <>
      <StepHeader
        eyebrow={eyebrow}
        title={t('Welcome back.')}
        body={
          several
            ? t('This account holds more than one lockbox. Pick the one to unlock here.')
            : t('Enter your master password to unlock on this device.')
        }
      />

      <div className={`${COLUMN} mt-9`}>
        <AccountVaults
          files={account.files}
          selectedId={account.selectedId}
          onSelect={setupAccountSelect}
        />

        <div className="mt-6">
          <Masterpass
            placeholder={t('Master password')}
            testid="drive-password-input"
            error={error}
            disabled={busy}
            onEnter={unlock}
            onChange={change}
          />
        </div>

        <div className="mt-8">
          <Button block testid="drive-unlock-button" loading={busy} onClick={unlock}>
            {busy ? t('Unlocking & syncing…') : t('Unlock & sync')}
          </Button>
        </div>
      </div>

      {links}
    </>
  )
}
