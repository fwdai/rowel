import { useTranslation } from 'react-i18next'
import { useApp } from '@/store'
import { syncProvider } from '@/api/sync'
import { providerName } from '@/api/provider'
import SettingsGroup from '@/components/elements/SettingsGroup'
import SettingsNavRow, { type StatusTone } from '@/components/elements/SettingsNavRow'
import { CloudGlyph, DiskGlyph, NearbyGlyph } from '@/components/Main/icons'
import { useSubpage } from '../../sectionNav'
import Summary from './Summary'

// Sync & backup, as an overview: where the vault stands, then one row per way
// of syncing it or keeping a copy, each saying where it stands and opening a
// page of its own. Nothing is switched on or connected from here.
export default function Sync() {
  const { t } = useTranslation()
  const { open } = useSubpage()
  const sync = useApp(state => state.sync)
  const nearby = useApp(state => state.nearby)
  const provider = syncProvider(sync)

  // A run in flight outranks the last one's failure, as on the account's card.
  const [cloudStatus, cloudTone]: [string, StatusTone] = sync.inProgress
    ? [t('Syncing…'), 'neutral']
    : sync.error !== null
      ? [t('Last attempt failed'), 'bad']
      : provider
        ? [providerName(provider), 'good']
        : [t('Not connected'), 'neutral']

  return (
    <>
      <Summary />

      <SettingsGroup label={t('Sync')}>
        {/* Only where this build can do it at all. */}
        {nearby?.available && (
          <SettingsNavRow
            icon={<NearbyGlyph size={16} />}
            iconActive={nearby.enabled}
            label={t('Nearby devices')}
            description={t('Directly between your devices on the same network. No account needed.')}
            status={nearby.enabled ? t('On') : t('Off')}
            statusTone={nearby.enabled ? 'good' : 'neutral'}
            onClick={() => open({ key: 'sync-nearby' })}
            testid="settings-sync-open-nearby"
          />
        )}
        <SettingsNavRow
          icon={<CloudGlyph size={16} />}
          iconActive={sync.configured}
          label={t('Your cloud storage')}
          description={t('Through storage you already own, such as Google Drive or Dropbox.')}
          status={cloudStatus}
          statusTone={cloudTone}
          onClick={() => open({ key: 'sync-cloud' })}
          testid="settings-sync-open-cloud"
        />
      </SettingsGroup>

      <SettingsGroup label={t('Backup')}>
        <SettingsNavRow
          icon={<DiskGlyph size={16} />}
          label={t('Encrypted backup')}
          description={t('A file you keep yourself, sealed with your master password.')}
          onClick={() => open({ key: 'sync-backup' })}
          testid="settings-sync-open-backup"
        />
      </SettingsGroup>
    </>
  )
}
