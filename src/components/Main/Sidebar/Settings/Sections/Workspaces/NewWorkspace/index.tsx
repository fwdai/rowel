import { useTranslation } from 'react-i18next'
import { PlusGlyph } from '@/components/Main/icons'
import { useSubpage } from '../../../sectionNav'
import AddRow from '../AddRow'

// The way to one more workspace, made here from nothing.
export default function NewWorkspace() {
  const { t } = useTranslation()
  const { open } = useSubpage()

  return (
    <AddRow
      icon={<PlusGlyph />}
      label={t('New lockbox')}
      description={t('A separate encrypted lockbox for work, family or a client')}
      onClick={() => open({ key: 'new-workspace' })}
      testid="workspace-new-row"
    />
  )
}
