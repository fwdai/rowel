import { useTranslation } from 'react-i18next'
import { useApp } from '@/store'
import { syncProvider } from '@/api/sync'
import { providerName } from '@/api/provider'
import { cloudStanding } from '@/lib/syncView'
import SettingsNavRow, { type StatusTone } from '@/components/elements/SettingsNavRow'
import { CloudGlyph } from '@/components/Main/icons'
import { useSubpage } from '../../sectionNav'

// The cloud account's standing, as its card's pill says it (`cloudStanding`),
// but for the one that needs no words: an account in sync is named instead.
export default function CloudRow() {
  const { t } = useTranslation()
  const { open } = useSubpage()
  const sync = useApp(state => state.sync)
  const provider = syncProvider(sync)

  const { tone, label } = cloudStanding(sync)
  const [status, statusTone]: [string, StatusTone] =
    tone === 'good' && provider
      ? [providerName(provider), 'good']
      : [t(label), tone === 'bad' ? 'bad' : 'neutral']

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
