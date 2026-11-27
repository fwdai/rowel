import { useRef } from 'react'
import type { EntryMeta, EntryType } from '@/api/types'
import { useSwapped } from '@/hooks/useSwapped'
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
export default function Show({ entry, type, editing }: Props) {
  const { kindType, current, failed, retry } = useShown(entry, type)
  const anchor = useRef<HTMLDivElement>(null)
  const ready = !entry || !!current || !!entry.deletedAt
  const { shown, settling } = useSwapped(
    { entry, revealed: current, editing: !!editing },
    `${entry?.id ?? `new-${kindType}`}:${!!editing}`,
    ready,
    anchor
  )

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
    return (
      <Read
        key={shown.entry.id}
        entry={shown.entry}
        revealed={shown.revealed}
        settling={settling}
      />
    )
  }

  return (
    <div
      ref={anchor}
      data-testid="entry-pane"
      className="contents"
      inert={settling && !refused}
    >
      {face()}
    </div>
  )
}
