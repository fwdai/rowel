import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import SettingsNavRow from '@/components/elements/SettingsNavRow'
import { CARD } from '@/components/elements/tokens'
import { DownloadGlyph } from '@/components/Main/icons'
import { useSubpage } from '../../../sectionNav'

// What a saved backup is for. Restoring one makes a lockbox, on the page
// Lockboxes offers for it too; from here it opens over this one, so Back leads
// to the backup page it was asked from.
export default function RestoreRow() {
  const { t } = useTranslation()
  const { open } = useSubpage()

  return (
    <div className={cx(CARD, 'mt-3')}>
      <SettingsNavRow
        icon={<DownloadGlyph size={16} />}
        label={t('Restore a backup')}
        description={t('A backup file and the master password it was sealed with')}
        onClick={() => open({ key: 'restore-backup' })}
        testid="settings-backup-restore"
      />
    </div>
  )
}
