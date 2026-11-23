import { useTranslation } from 'react-i18next'
import { DiskGlyph } from '@/components/Main/icons'
import { useSubpage } from '../../../sectionNav'
import AddRow from '../AddRow'

// The way to one more workspace kept as a backup file: a `.rowel` saved from
// Settings, here or on another device, restored beside the lockboxes already
// on this one.
export default function RestoreBackupRow() {
  const { t } = useTranslation()
  const { open } = useSubpage()

  return (
    <AddRow
      icon={<DiskGlyph size={16} />}
      label={t('Restore a backup')}
      description={t('A backup file and the master password it was sealed with')}
      onClick={() => open({ key: 'restore-backup' })}
      testid="workspace-backup-open"
    />
  )
}
