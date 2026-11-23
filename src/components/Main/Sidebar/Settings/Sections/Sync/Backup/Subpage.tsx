import SubpageScroller from '../../../SubpageScroller'
import Saves from './Saves'
import RestoreRow from './RestoreRow'

// Sync & backup › Encrypted backup: saving one, and the way to restoring one.
export default function BackupSubpage() {
  return (
    <SubpageScroller testid="settings-backup">
      {/* No heading over the card: the page's own title is the heading. */}
      <Saves />
      <RestoreRow />
    </SubpageScroller>
  )
}
