import { useTranslation } from 'react-i18next'
import { cx } from '@/utils/cx'
import SettingsNavRow from '@/components/elements/SettingsNavRow'
import { CARD } from '@/components/elements/tokens'
import { DownloadGlyph } from '@/components/Main/icons'
import { useSectionNav } from '../../../sectionNav'

// What a saved backup is for. Restoring one makes a lockbox, so its page is
// under Lockboxes, and this row goes there rather than restoring in place.
export default function RestoreRow() {
  const { t } = useTranslation()
  const go = useSectionNav()

  return (
    <div className={cx(CARD, 'mt-3')}>
      <SettingsNavRow
        icon={<DownloadGlyph size={16} />}
        label={t('Restore a backup')}
        description={t('A backup file and the master password it was sealed with')}
        onClick={() => go('workspaces', { key: 'restore-backup' })}
        testid="settings-backup-restore"
      />
    </div>
  )
}
