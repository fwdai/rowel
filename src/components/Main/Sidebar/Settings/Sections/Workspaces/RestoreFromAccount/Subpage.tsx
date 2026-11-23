import { CARD } from '@/components/elements/tokens'
import SubpageScroller from '../../../SubpageScroller'
import RestoreFromAccount from '.'

// Lockboxes › Restore from a cloud account: the account's sign-in and the
// pick of one of its lockboxes.
export default function RestoreSubpage() {
  return (
    <SubpageScroller testid="settings-restore-workspace">
      {/* No heading over the card: the page's own title is the heading. */}
      <div className={CARD}>
        <RestoreFromAccount />
      </div>
    </SubpageScroller>
  )
}
