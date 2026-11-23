import { useTranslation } from 'react-i18next'
import Button from '@/components/elements/Button'
import SettingsRow from '@/components/elements/SettingsRow'
import { CARD } from '@/components/elements/tokens'
import { DiskGlyph } from '../../../../icons'

interface Props {
  active: boolean
  disabled: boolean
  onBackup: () => void
}

// A backup of our own, apart from the app tiles: it is not an export to parse
// but a sealed file, and asks for the password it was sealed with.
export default function BackupCard({ active, disabled, onBackup }: Props) {
  const { t } = useTranslation()
  return (
    <div className={CARD}>
      <SettingsRow
        icon={<DiskGlyph size={16} />}
        iconActive={active}
        label={t('Restore from a backup')}
        description={t('A backup file and the master password it was sealed with')}
        control={
          <Button
            variant="pale"
            size="md"
            disabled={disabled}
            onClick={onBackup}
            testid="import-tile-swftx"
          >
            {t('Restore…')}
          </Button>
        }
      />
    </div>
  )
}
