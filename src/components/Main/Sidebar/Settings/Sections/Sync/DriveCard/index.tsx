import { useTranslation } from 'react-i18next'
import {
  useApp,
  restoreWorkspaceFromDrive,
  setupDriveSelect,
  switchWorkspaceDriveAccount
} from '@/store'
import { cx } from '@/utils/cx'
import { CARD, HERO_WASH, META } from '@/components/elements/tokens'
import DriveRestoreForm from '@/components/elements/DriveRestoreForm'
import GoogleDriveMark from '@/components/elements/GoogleDriveMark'
import StatusPill from './StatusPill'
import Control from './Control'
import Stats from './Stats'
import Footer from './Footer'

interface Props {
  // Waiting on Google: the probe out with the browser, or the account's
  // answer with the backend.
  probing: boolean
  // The account holds only other vaults, and they are offered to restore.
  offered: boolean
  restoring: boolean
}

// Google Drive as the one account this section is about: where it stands, the
// control that moves it on, and — once connected — what it holds and the way out.
export default function DriveCard({ probing, offered, restoring }: Props) {
  const { t } = useTranslation()
  const sync = useApp(state => state.sync)
  const drive = useApp(state => state.setupDrive)

  const description =
    sync.pending || probing
      ? t('Waiting for Google…')
      : offered
        ? t('This account already holds a lockbox')
        : sync.configured
          ? t('Connected')
          : t('Not connected')

  const lastSynced = sync.inProgress
    ? t('Syncing…')
    : sync.error === null
      ? t('Up to date')
      : t('Last attempt failed')

  return (
    <div data-testid="settings-drive-row" className={cx(CARD, HERO_WASH)}>
      <div className="flex items-center gap-4 p-5">
        <div className="grid h-11 w-11 flex-none place-items-center rounded-lg bg-accent-soft text-accent">
          <GoogleDriveMark size={22} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-md font-semibold tracking-display text-text">
              {t('Google Drive')}
            </span>
            <StatusPill sync={sync} />
          </div>
          <div className={cx('mt-0.5', META)}>{description}</div>
        </div>
        <div className="flex-none">
          <Control probing={probing} offered={offered} restoring={restoring} />
        </div>
      </div>

      {offered && (
        <div className="flex flex-col gap-3 px-5 pb-5">
          <p data-testid="settings-drive-found" className="text-sm text-text2">
            {t(
              'Restore it here to sync the same data on this device. The lockbox open now is kept alongside it.'
            )}
          </p>
          <DriveRestoreForm
            files={drive.files}
            selectedId={drive.selectedId}
            onSelect={setupDriveSelect}
            busy={restoring}
            onRestore={restoreWorkspaceFromDrive}
            onSwitchAccount={switchWorkspaceDriveAccount}
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
