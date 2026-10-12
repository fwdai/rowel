import { useTranslation } from 'react-i18next'
import Button from '@/components/elements/Button'
import FoundFileCard from '@/components/elements/FoundFileCard'
import { selectedRemotePack, useApp } from '@/store'
import { providerName } from '@/api/provider'
import StepHeader from '../shared/StepHeader'
import { COLUMN, ACTIONS } from '../shared/layout'
import { describeRemotePack } from '@/utils/remotePack'
import { useDates } from '@/hooks/useDates'

interface Props {
  /** Open the data that is already there, rather than the one just chosen. */
  onUnlockExisting: () => void
}

// The account already holds a vault, so the fresh start the user was on stops
// here. One way forward, on purpose: every device connected to an account
// syncs the vaults it holds. Starting a second vault beside the first is how a
// user's data got forked across two packs when they meant one, so it is not
// offered — a new vault is made from Settings › Workspaces on a device that is
// already connected, and every other device then picks it up.
export default function Conflict({ onUnlockExisting }: Props) {
  const { t } = useTranslation()
  const dates = useDates()
  const file = useApp(selectedRemotePack)
  const provider = useApp(state => state.setupAccount.provider) ?? 'googleDrive'

  return (
    <>
      <StepHeader
        eyebrow={t('Hold on')}
        tone="warn"
        title={t('This {{provider}} already has Rowel data', { provider: providerName(provider) })}
        body={t(
          "It's sealed with its own master password. Unlock that vault here and this device will sync the same data as your others."
        )}
      />

      {file && (
        <div className={`${COLUMN} mt-9`}>
          <FoundFileCard
            where="cloud"
            testid="setup-conflict-file"
            name={t('Rowel vault')}
            meta={describeRemotePack(file, dates)}
            encrypted
          />
        </div>
      )}

      <div className={ACTIONS}>
        <Button block testid="setup-unlock-existing-button" onClick={onUnlockExisting}>
          {t('Unlock it instead')}
        </Button>
      </div>
    </>
  )
}
