import { useTranslation } from 'react-i18next'
import { useApp } from '@/store'
import { syncProvider } from '@/api/sync'
import { providerName } from '@/api/provider'
import { cloudView } from '@/lib/syncView'
import SettingsNavRow, { type StatusTone } from '@/components/elements/SettingsNavRow'
import { CloudGlyph } from '@/components/Main/icons'
import { useSubpage } from '../../sectionNav'

// The cloud account's standing (`cloudView`, as the chip and the account
// card's pill read it): in flight, failed, connected (by the provider's name),
// or not connected.
export default function CloudRow() {
  const { t } = useTranslation()
  const { open } = useSubpage()
  const sync = useApp(state => state.sync)
  const provider = syncProvider(sync)

  const tone = cloudView(sync)?.tone
  const [status, statusTone]: [string, StatusTone] =
    tone === 'loading'
      ? [t('Syncing…'), 'neutral']
      : tone === 'bad'
        ? [t('Last attempt failed'), 'bad']
        : provider
          ? [providerName(provider), 'good']
          : [t('Not connected'), 'neutral']

  return (
    <SettingsNavRow
      icon={<CloudGlyph size={16} />}
      iconActive={sync.configured}
      label={t('Your cloud storage')}
      description={t('Through storage you already own, such as Google Drive or Dropbox.')}
      status={status}
      statusTone={statusTone}
      onClick={() => open({ key: 'sync-cloud' })}
      testid="settings-sync-open-cloud"
    />
  )
}
