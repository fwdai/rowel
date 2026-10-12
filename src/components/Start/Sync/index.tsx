import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import Button from '@/components/elements/Button'
import { META_TYPE } from '@/components/elements/tokens'
import ProviderButtons from '@/components/elements/ProviderButtons'
import { useApp, setupAccountReset, connectAccount } from '@/store'
import { providerName, type Provider } from '@/api/provider'
import StepHeader from '../shared/StepHeader'
import SpinnerCard from '../shared/SpinnerCard'
import { COLUMN, STACK, FOOTNOTE } from '../shared/layout'
import { describeError } from '@/api/errors'

interface Props {
  /** Create the data here and now, with sync on if consent already landed. */
  onCreate: () => Promise<void>
  /** The probe found data in this account: that is a decision, not a step. */
  onConflict: () => void
}

// Step two of a fresh start: where the backup lives. One question, two
// answers, and nothing else on screen that could be mistaken for a third.
// Connecting is also how we find out whether the account already holds data
// — so the answer to the probe, not the press, is what decides where this goes
// next. One connect per provider; whichever is pressed is the one probed.
export default function Sync({ onCreate, onConflict }: Props) {
  const { t } = useTranslation()
  const account = useApp(state => state.setupAccount)
  const [error, setError] = useState<string | null>(null)
  // Creating is in flight. Both buttons go inert on it: a second press would
  // ask the backend to create again, and while the backend now refuses the
  // overlap, the refusal would land here as an error over a create that is
  // about to succeed.
  const [busy, setBusy] = useState(false)
  // The probe answers once. Without this the effect would re-fire on every
  // render the status outlives — and create the data twice.
  const settled = useRef(false)

  const fail = (message: string) => {
    setError(message)
    setupAccountReset()
  }

  // One entry point for both ways a create starts (a press, or the probe
  // coming back empty), so neither can start one while the other is running.
  const create = useCallback(
    (onError: (message: string) => void) => {
      if (busy) return
      setBusy(true)
      setError(null)
      onCreate().catch((err: unknown) => {
        setBusy(false)
        onError(describeError(err) || t('Something went wrong'))
      })
    },
    [busy, onCreate, t]
  )

  useEffect(() => {
    if (settled.current) return

    if (account.status === 'empty') {
      // A bare account: nothing to weigh up, so this is the last step.
      settled.current = true
      create(message => {
        settled.current = false
        fail(message)
      })
    } else if (account.status === 'found') {
      settled.current = true
      onConflict()
    } else if (account.status === 'error') {
      fail(account.error || t('Something went wrong'))
    }
  }, [account.status, account.error, create, onConflict, t])

  const connect = (provider: Provider) => {
    if (busy) return
    setError(null)
    connectAccount(provider)
  }

  if (account.status === 'pending')
    return (
      <>
        <StepHeader
          eyebrow={t('Get started · 2 of 2')}
          busy
          title={t('Connecting to {{provider}}…', {
            provider: providerName(account.provider ?? 'googleDrive')
          })}
        />
        <div className={`${COLUMN} mt-8`}>
          <SpinnerCard
            testid="setup-drive-spinner"
            caption={t('Checking for existing data first')}
          />
        </div>
      </>
    )

  return (
    <>
      <StepHeader
        eyebrow={t('Get started · 2 of 2')}
        title={t('Back up to the cloud')}
        body={t("Your data syncs to your own cloud storage, so a lost device isn't lost secrets.")}
      />

      <div className={`${COLUMN} mt-8 ${STACK}`}>
        <ProviderButtons
          variant="primary"
          block
          disabled={busy}
          onPick={connect}
          testid="setup"
          className={STACK}
        />
        <Button
          block
          variant="pale"
          testid="setup-skip-drive-button"
          loading={busy}
          onClick={() => create(setError)}
        >
          {t('Keep it on this device')}
        </Button>
      </div>

      {/* The two things worth knowing before pressing, as a footnote rather
          than a list: a list of rows here read as three more things to press. */}
      <p
        data-testid={error ? 'setup-drive-error' : undefined}
        className={`${FOOTNOTE} ${META_TYPE} ${error ? 'text-bad' : 'text-text2'}`}
      >
        {error ??
          `${t('Encrypted before it leaves this device')} · ${t('Switch off any time in Settings')}`}
      </p>
    </>
  )
}
