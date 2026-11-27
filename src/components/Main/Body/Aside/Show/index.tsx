import type { EntryMeta, EntryType } from '@/api/types'
import { useHeld } from '@/hooks/useHeld'
import Read from './Read'
import Edit from './Edit'
import RevealError from './RevealError'
import { useShown } from './useShown'

interface Props {
  /** Absent while creating: there is nothing saved to read yet. */
  entry?: EntryMeta
  /** The kind being created; an edit takes the entry's own. */
  type?: EntryType
  editing?: boolean
}

// The detail pane's one entry surface. The decrypt lives in `useShown` rather
// than in either mode, so entering and leaving edit doesn't re-fetch the
// secrets and blank the rows on the way through.
//
// One instance serves every entry in turn, so that the pane can keep the entry
// it was showing until the next one's secrets are in hand, and then change
// once (`useHeld`): a header without its rows, or a copy button with nothing
// to copy yet, is never drawn on the way.
export default function Show({ entry, type, editing }: Props) {
  const { kindType, current, failed, retry } = useShown(entry, type)
  // A draft and a tombstone have no secrets to wait for: `reveal_entry` serves
  // neither. A tombstone never gets to the editor (`editEntry` refuses it), so
  // an editor is only ever seeded from a reveal that has landed.
  const ready = !entry || !!current || !!entry.deletedAt
  const shown = useHeld({ entry, revealed: current, editing: !!editing }, ready)

  if (!kindType) return null

  // A refused reveal is the same dead end in both faces: no rows to read and
  // no secrets to seed a draft from. Say so, and offer the way back.
  const refused = failed && !!entry

  const face = () => {
    if (refused) return <RevealError entry={entry} onRetry={retry} />
    if (!shown) return null
    // Keyed per entry: each editing session starts from a fresh draft. A new
    // entry has no id to key on, so it is keyed by its kind — choosing another
    // kind (the picker again, or a scan that recognized a different one) is a
    // different draft, not the same one with other rows on it.
    if (shown.editing)
      return (
        <Edit
          key={shown.entry?.id ?? `new-${kindType}`}
          type={shown.entry?.type ?? kindType}
          revealed={shown.revealed}
        />
      )
    if (!shown.entry) return null
    // Keyed per entry too, so the next one starts from a fresh surface rather
    // than one still holding the last one's open menu and delete error.
    return (
      <Read
        key={shown.entry.id}
        entry={shown.entry}
        revealed={shown.revealed}
        settling={!ready}
      />
    )
  }

  return (
    // No box of its own (`contents`): what it draws lays itself out in the
    // pane. It is here to keep an entry that is on its way out from being
    // pressed — the held entry only: a refusal takes its place, and its two
    // buttons are the way on.
    <div className="contents" inert={!ready && !refused}>
      {face()}
    </div>
  )
}
