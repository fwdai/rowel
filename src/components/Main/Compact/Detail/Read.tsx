import type { Entry, EntryMeta } from '@/api/types'
import { setNoEntry } from '@/store'
import { cx } from '@/utils/cx'
import Body from '../../Body/Aside/Show/Body'
import Eyebrow from '../../Body/Aside/Show/Eyebrow'
import Favorite from '../../Body/Aside/Show/Favorite'
import Identity from '../../Body/Aside/Show/Identity'
import { useDelete } from '../../Body/Aside/Show/useDelete'
import { HEADER_ACTION, PRIMARY_CLEARANCE } from '../chrome'
import { useSwipeBack } from '../useSwipeBack'
import NavRow from './NavRow'
import PrimaryAction from './PrimaryAction'

interface Props {
  entry: EntryMeta
  /** The decrypted entry, or null while `revealEntry` is still in flight. */
  revealed: Entry | null
}

/**
 * Reading one entry, on a phone: a nav row, the entry's own identity at the
 * size a 390px screen can afford, the kind's field set, the footer, and the
 * primary action pinned where a thumb is.
 *
 * Every part below the nav row is the desktop's, unchanged — the whole block
 * under the header is literally `Show/Body`. What differs is where they are
 * placed and how much room they have: the scroller declares itself a
 * `@container`, and the rows fold themselves (see fields/Row).
 *
 * The reveal is the screen's, not this face's (`Compact/Entry`), so entering
 * and leaving edit never re-fetches the secrets.
 */
export default function Read({ entry, revealed }: Props) {
  const { error, remove } = useDelete(entry.id)
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
        {/* The screen has no title bar, so the entry names itself here — a
            60px tile beside the title, with the eyebrow over it — and the star
            sits at the row's far end: it is about this entry, so it belongs
            beside the entry's name rather than up among the screen's controls.
            A tombstone has no star to set. */}
        <div className="flex items-center gap-2">
          <Identity
            entry={entry}
            tile="h-[60px] w-[60px]"
            glyph={28}
            className="flex min-w-0 flex-1 items-center gap-3.5"
          >
            <Eyebrow
              entry={entry}
              revealed={revealed}
              className="mt-1 flex items-center gap-2 truncate whitespace-nowrap"
            />
          </Identity>
          {!entry.deletedAt && <Favorite entry={entry} className={`${HEADER_ACTION} flex-none`} />}
        </div>

        <Body entry={entry} revealed={revealed} error={error} />
      </div>

      {/* `reveal_entry` does not serve deleted rows, so a tombstone has no
          secret to offer and nothing to offer it with. */}
      {!entry.deletedAt && <PrimaryAction entry={entry} revealed={revealed} />}
    </div>
  )
}
