import { useTranslation } from 'react-i18next'
import SettingsNavRow from '@/components/elements/SettingsNavRow'
import { CARD } from '@/components/elements/tokens'
import { FileArchiveGlyph } from '@/components/Main/icons'
import { useSectionNav } from '../../../sectionNav'
import SubpageScroller from '../../../SubpageScroller'
import BackupRow from './BackupRow'

// Sync & backup › Encrypted backup: saving one here, and the way to where one
// is restored, which is Import's to do.
export default function BackupSubpage() {
  const { t } = useTranslation()
  const go = useSectionNav()

  return (
    <SubpageScroller testid="settings-backup">
      {/* No heading over the card: the page's own title is the heading. */}
      <div className={CARD}>
        <BackupRow />
        <SettingsNavRow
          icon={<FileArchiveGlyph size={16} />}
          label={t('Restore a backup')}
          description={t('A backup file and the master password it was sealed with')}
          onClick={() => go('import')}
          testid="settings-backup-restore"
        />
      </div>
    </SubpageScroller>
  )
}
