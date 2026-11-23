import { useTranslation } from 'react-i18next'
import { CloudGlyph } from '@/components/Main/icons'
import { useSubpage } from '../../../sectionNav'
import AddRow from '../AddRow'

// The way to one more workspace that already exists in a cloud account. Its
// own page, so the account's Connect buttons are never read as connecting
// this vault, which is what the same buttons do under Sync & backup.
export default function RestoreRow() {
  const { t } = useTranslation()
  const { open } = useSubpage()

  return (
    <AddRow
      icon={<CloudGlyph size={16} />}
      label={t('Restore from a cloud account')}
      description={t('Bring in a lockbox that already exists in your cloud storage')}
      onClick={() => open({ key: 'restore-workspace' })}
      testid="workspace-restore-open"
    />
  )
}
