import type { Entry, EntryMeta } from '@/api/types'
import { setNoEntry } from '@/store'
import { kindOf } from '@/kinds'
import { cx } from '@/utils/cx'
import Body from '../../Body/Aside/Show/Body'
import Eyebrow from '../../Body/Aside/Show/Eyebrow'
import Favorite from '../../Body/Aside/Show/Favorite'
import Identity from '../../Body/Aside/Show/Identity'
import { useDelete } from '../../Body/Aside/Show/useDelete'
import { HEADER_ACTION, PRIMARY_CLEARANCE } from '../chrome'
import { useSwipeBack } from '../useSwipeBack'
import Meta from './Meta'
import NavRow from './NavRow'
import PrimaryAction from './PrimaryAction'

interface Props {
  entry: EntryMeta
  /** The decrypted entry, or null while `revealEntry` is still in flight. */
  revealed: Entry | null
}

/**
 * Reading one entry, on a phone: a nav row, the entry's own identity, the
 * kind's field set, the files, tags and dates, and the primary action pinned
 * where a thumb is.
 *
 * Every part below the nav row is the desktop's, unchanged — the whole block
 * under the header is literally `Show/Body`. What differs is where they are
 * placed and how much room they have: the scroller declares itself a
 * `@container`, the rows fold themselves (see fields/Row), and the footer's
 * facts are laid out by `Meta` rather than the desktop's strip.
 *
 * The reveal is the screen's, not this face's (`Compact/Entry`), so entering
 * and leaving edit never re-fetches the secrets.
 */
export default function Read({ entry, revealed }: Props) {
  const { error, remove } = useDelete(entry.id)
  const kind = kindOf(entry.type)
  // The edge swipe is the nav row's back control, by gesture.
  const swipe = useSwipeBack(setNoEntry)

  return (
    // `relative`: what the bottom action and its fade are pinned to.
    <div
      {...swipe}
      data-testid="entry-screen"
      className="relative flex min-h-0 flex-1 flex-col animate-sheet bg-screen text-text"
    >
      <NavRow entry={entry} onDelete={remove} />

      <div
        className={cx(
          '@container min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 pt-1',
          PRIMARY_CLEARANCE
        )}
      >
        {/* The screen has no title bar, so the entry names itself here: one
            44px tile beside the title, with the star — a 44px header action —
            at the row's far end, so the two ends of the row weigh the same. A
            line under the title only where the kind has something to add that
            no row says (a document's type); the kind's name and the host are
            not it, since the rows below are what the entry is and where it
            belongs. A tombstone has no star to set. */}
        <div className="flex items-center gap-2">
          <Identity
            entry={entry}
            tile="h-11 w-11"
            glyph={22}
            className="flex min-w-0 flex-1 items-center gap-3"
          >
            {kind.eyebrow && (
              <Eyebrow entry={entry} revealed={revealed} bare className="mt-0.5 truncate" />
            )}
          </Identity>
          {!entry.deletedAt && <Favorite entry={entry} className={`${HEADER_ACTION} flex-none`} />}
        </div>

        <Body entry={entry} revealed={revealed} error={error} Footer={Meta} />
      </div>

      {/* `reveal_entry` does not serve deleted rows, so a tombstone has no
          secret to offer and nothing to offer it with. */}
      {!entry.deletedAt && <PrimaryAction entry={entry} revealed={revealed} />}
    </div>
  )
}
