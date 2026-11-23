import { CARD } from '@/components/elements/tokens'
import type { Subpage } from '../../../subpages'
import SubpageScroller from '../../../SubpageScroller'
import RestoreBackup from '.'

// Restore from a backup, under Lockboxes, Sync & backup or Import: the file,
// and the master password it was sealed with.
export default function RestoreBackupSubpage({
  subpage
}: {
  subpage: Extract<Subpage, { key: 'restore-backup' }>
}) {
  return (
    <SubpageScroller testid="settings-restore-backup">
      {/* No heading over the card: the page's own title is the heading. */}
      <div className={CARD}>
        <RestoreBackup chosen={subpage.path ?? null} />
      </div>
    </SubpageScroller>
  )
}
