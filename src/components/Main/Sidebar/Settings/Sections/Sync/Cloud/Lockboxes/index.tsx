import { useTranslation } from 'react-i18next'
import { useApp } from '@/store'
import { syncProvider } from '@/api/sync'
import SettingsGroup from '@/components/elements/SettingsGroup'
import { useAccountLockboxes } from '../useAccountLockboxes'
import LocalRow from './LocalRow'
import RemoteRow from './RemoteRow'

// Under the account's card: each lockbox and where it stands with the account
// (`accountLockboxes`), out of the same list the card's figure counts.
//
// Drawn only once it says more than the card does. With the open lockbox alone
// in the account and nothing else on the device, it would be a list of the one
// lockbox the card is already about. With no account it is empty, and the
// choice of provider stands alone.
export default function Lockboxes() {
  const { t } = useTranslation()
  const provider = useApp(state => syncProvider(state.sync))
  const lockboxes = useAccountLockboxes()
  const others = lockboxes.some(lockbox => lockbox.standing === 'account' || !lockbox.open)

  if (!provider || !others) return null

  return (
    <div data-testid="settings-cloud-lockboxes">
      <SettingsGroup label={t('Lockboxes')}>
        {lockboxes.map(lockbox =>
          lockbox.standing === 'account' ? (
            <RemoteRow key={lockbox.pack.id} pack={lockbox.pack} />
          ) : (
            <LocalRow key={lockbox.workspace.id} lockbox={lockbox} provider={provider} />
          )
        )}
      </SettingsGroup>
    </div>
  )
}
