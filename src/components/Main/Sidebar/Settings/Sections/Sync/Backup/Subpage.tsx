import { useTranslation } from 'react-i18next'
import Note from '@/components/elements/Note'
import { CARD } from '@/components/elements/tokens'
import SubpageScroller from '../../../SubpageScroller'
import BackupRow from './BackupRow'

// Sync & backup › Encrypted backup: saving one, and what a saved one is for.
// A backup is a whole sealed database, which only a fresh install can take, so
// the page says so rather than offering a restore an open vault cannot do.
export default function BackupSubpage() {
  const { t } = useTranslation()

  return (
    <SubpageScroller testid="settings-backup">
      {/* No heading over the card: the page's own title is the heading. */}
      <div className={CARD}>
        <BackupRow />
      </div>
      <Note className="px-1 pt-2.5" testid="settings-backup-restore-note">
        {t('Backups restore a fresh install and are not imported into an existing vault.')}
      </Note>
    </SubpageScroller>
  )
}
