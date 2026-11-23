import SubpageScroller from '../../../SubpageScroller'
import Saves from './Saves'

// Sync & backup › Back up the vault: a file of each lockbox to save. Bringing
// one back is the overview's other row, not a step from here.
export default function BackupSubpage() {
  return (
    <SubpageScroller testid="settings-backup">
      {/* No heading over the card: the page's own title is the heading. */}
      <Saves />
    </SubpageScroller>
  )
}
