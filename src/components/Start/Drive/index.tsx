import { useState, type ChangeEvent } from 'react'
import { useTranslation } from 'react-i18next'
import Masterpass from '@/components/elements/Masterpass'
import Button from '@/components/elements/Button'
import DriveVaults from '@/components/elements/DriveVaults'
import { isMobile } from '@/lib/platform'
import type { UnlockResult } from '@/api/types'
import { setupRestoreFromDrive } from '@/api/setup'
import {
  useApp,
  selectedDriveFile,
  setupDriveSelect,
  connectDrive,
  switchDriveAccount
} from '@/store'
import StepHeader from '../shared/StepHeader'
import SpinnerCard from '../shared/SpinnerCard'
import TextLink from '../shared/TextLink'
import { COLUMN, ACTIONS, FOOTNOTE } from '../shared/layout'
import { unsealError } from '../shared/errors'

interface Props {
  /** Nothing in this account: go and make some, with sync already wired up. */
  onStartFresh: () => void
  /** Desktop's other route in, for a Drive that is not where the data is. */
  onUseFile: () => void
  onRestored: (result: UnlockResult) => Promise<void>
}

// Restoring from Google Drive. Everything on screen follows the probe in the
// store — pending while consent is out with the browser, then one of found,
// empty or failed — so a result that arrives while the user is reading simply
// redraws the screen.
export default function Drive({ onStartFresh, onUseFile, onRestored }: Props) {
  const { t } = useTranslation()
  const drive = useApp(state => state.setupDrive)
  const file = useApp(selectedDriveFile)
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
    setupRestoreFromDrive(password, file.id)
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
      <TextLink testid="drive-switch-account" onClick={switchDriveAccount}>
        {t('Switch account')}
      </TextLink>
      {!isMobile && (
        <TextLink testid="drive-use-file" onClick={onUseFile}>
          {t('Use a backup file instead')}
        </TextLink>
      )}
    </div>
  )

  if (drive.status === 'error')
    return (
      <>
        <StepHeader
          eyebrow={t('Restore · Google Drive')}
          tone="bad"
          title={t('Google Drive did not answer')}
          body={drive.error || t('Something went wrong')}
        />
        <div className={ACTIONS}>
          <Button block testid="drive-retry-button" onClick={connectDrive}>
            {t('Try again')}
          </Button>
        </div>
        {links}
      </>
    )

  if (drive.status === 'empty')
    return (
      <>
        <StepHeader
          eyebrow={t('Restore · Google Drive')}
          title={t('Nothing here yet')}
          body={t('This Google account has no Rowel data. Start fresh and it will sync here.')}
        />
        <div className={ACTIONS}>
          <Button block testid="drive-start-fresh-button" onClick={onStartFresh}>
            {t('Start fresh')}
          </Button>
        </div>
        {links}
      </>
    )

  if (drive.status !== 'found')
    return (
      <>
        <StepHeader
          eyebrow={t('Restore · Google Drive')}
          busy
          title={t('Looking for your data…')}
        />
        <div className={`${COLUMN} mt-9`}>
          <SpinnerCard testid="drive-spinner" caption={t('Waiting for Google…')} />
        </div>
      </>
    )

  // What the account holds decides the wording: a choice to make, or one vault
  // to unlock. `DriveVaults` draws whichever of the two it is.
  const several = drive.files.length > 1

  return (
    <>
      <StepHeader
        eyebrow={t('Restore · Google Drive')}
        title={t('Welcome back.')}
        body={
          several
            ? t('This account holds more than one lockbox. Pick the one to unlock here.')
            : t('Enter your master password to unlock on this device.')
        }
      />

      <div className={`${COLUMN} mt-9`}>
        <DriveVaults
          files={drive.files}
          selectedId={drive.selectedId}
          onSelect={setupDriveSelect}
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
