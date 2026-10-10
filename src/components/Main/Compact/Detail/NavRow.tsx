import type { EntryMeta } from '@/api/types'
import { editEntry, openSend, setNoEntry } from '@/store'
import Archived from '../../Body/Aside/Show/Archived'
import MoreMenu from '../../Body/Aside/Show/MoreMenu'
import { useListTitle } from '../../Body/ListColumn/useListTitle'
import BackButton from '../BackButton'
import NavBar from '../NavBar'
import { HEADER_ACTION } from '../chrome'

/**
 * The read screen's nav row: the way back on the left, one menu on the right,
 * no centred title — the entry names itself in the header below.
 *
 * The row used to carry the star and an Edit button too. Three controls in the
 * corner read as clutter beside the back control, so Edit went under the menu
 * (it is a step away, as iOS's own Edit often is) and the star moved down to
 * the entry's identity row, where it says what it is about — see `Read`.
 *
 * The back control carries the *previous* screen's name rather than the word
 * "Back", from the same hook the list root draws its title with, so the two can
 * never disagree about where this screen was pushed from.
 */
export default function NavRow({
  entry,
  onDelete
}: {
  entry: EntryMeta
  onDelete: () => void
}) {
  const from = useListTitle()

  return (
    <NavBar
      leading={<BackButton testid="compact-back" label={from} onClick={setNoEntry} />}
      trailing={
        // A tombstone has no editor and no menu — only Restore and the last
        // delete, which is exactly what the desktop offers too.
        entry.deletedAt ? (
          // The desktop's 28px control tier is under the touch minimum, so the
          // pair is raised to the 44px one the rest of this row sits at.
          <Archived entry={entry} className="h-11" />
        ) : (
          // Under the trigger rather than 32px down from its top: the nav row's
          // button is 44px tall, not 28px.
          <MoreMenu
            onDelete={onDelete}
            onEdit={() => editEntry()}
            onShare={() => openSend(entry.id)}
            className={HEADER_ACTION}
            menu="right-0 top-full mt-1"
          />
        )
      }
    />
  )
}
