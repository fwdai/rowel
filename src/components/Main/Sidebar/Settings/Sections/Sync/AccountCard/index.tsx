import { useTranslation } from 'react-i18next'
import {
  useApp,
  connectSync,
  restoreWorkspaceFromPending,
  setupAccountSelect,
  switchWorkspaceAccount
} from '@/store'
import { syncProvider } from '@/api/sync'
import { cx } from '@/utils/cx'
import { CARD, HERO_WASH } from '@/components/elements/tokens'
import AccountRestoreForm from '@/components/elements/AccountRestoreForm'
import ProviderButtons from '@/components/elements/ProviderButtons'
import Header from './Header'
import Stats from './Stats'
import Footer from './Footer'

interface Props {
  // Waiting on the provider: the probe out with the browser, or the account's
  // answer with the backend.
  probing: boolean
  // The account holds only other vaults, and they are offered to restore.
  offered: boolean
  restoring: boolean
}

// The one account this vault syncs through: where it stands, the control that
// moves it on, and — once connected — what it holds and the way out. Before
// there is one, the choice of provider; one account per workspace, so a vault
// on Drive moves to Dropbox by disconnecting and connecting the other.
export default function AccountCard({ probing, offered, restoring }: Props) {
  const { t } = useTranslation()
  const sync = useApp(state => state.sync)
  const account = useApp(state => state.setupAccount)

  // A connected vault says whose it is; a connect under way is with whichever
  // provider was pressed.
  const busy = sync.pending || probing || offered
  const provider = syncProvider(sync) ?? (busy ? account.provider : null)

  const lastSynced = sync.inProgress
    ? t('Syncing…')
    : sync.error === null
      ? t('Up to date')
      : t('Last attempt failed')

  return (
    <div data-testid="settings-drive-row" className={cx(CARD, HERO_WASH)}>
      <Header provider={provider} probing={probing} offered={offered} restoring={restoring} />

      {!sync.configured && !busy && (
        <ProviderButtons
          testid="settings"
          onPick={connectSync}
          disabled={sync.inProgress}
          className="flex flex-wrap gap-2 px-5 pb-5"
        />
      )}

      {offered && (
        <div className="flex flex-col gap-3 px-5 pb-5">
          <p data-testid="settings-drive-found" className="text-sm text-text2">
            {t(
              'Restore it here to sync the same data on this device. The vault open now stays as a workspace of its own.'
            )}
          </p>
          <AccountRestoreForm
            files={account.files}
            selectedId={account.selectedId}
            onSelect={setupAccountSelect}
            busy={restoring}
            onRestore={restoreWorkspaceFromPending}
            onSwitchAccount={switchWorkspaceAccount}
          />
        </div>
      )}

      {sync.configured && (
        <>
          <Stats lastSynced={lastSynced} />
          <Footer />
        </>
      )}
    </div>
  )
}
