import { useTranslation } from 'react-i18next'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsNavRow from '@/components/elements/SettingsNavRow'
import { DiskGlyph } from '@/components/Main/icons'
import { useSubpage } from '../../sectionNav'
import { useBackupStatus } from './Backup/status'
import Summary from './Summary'
import NearbyRow from './NearbyRow'
import CloudRow from './CloudRow'

// Sync & backup, as an overview: where the vault stands, then one row per way
// of syncing it or keeping a copy, each saying where it stands and opening a
// page of its own. Nothing is switched on or connected from here.
export default function Sync() {
  const { t } = useTranslation()
  const { open } = useSubpage()
  const backupStatus = useBackupStatus()

  return (
    <>
      <Summary />

      <SettingsGroup label={t('Sync')}>
        <NearbyRow />
        <CloudRow />
      </SettingsGroup>

      <SettingsGroup label={t('Backup')}>
        <SettingsNavRow
          icon={<DiskGlyph size={16} />}
          label={t('Encrypted backup')}
          description={t('A file you keep yourself, sealed with your master password.')}
          status={backupStatus}
          onClick={() => open({ key: 'sync-backup' })}
          testid="settings-sync-open-backup"
        />
      </SettingsGroup>
    </>
  )
}
