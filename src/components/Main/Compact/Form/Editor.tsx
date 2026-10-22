import type { Entry, EntryMeta, EntryType } from '@/api/types'
import { kindOf } from '@/kinds'
import Body from '../../Body/Aside/Show/Edit/Body'
import Title from '../../Body/Aside/Show/Edit/Title'
import Eyebrow from '../../Body/Aside/Show/Eyebrow'
import { useDraft } from '../../Body/Aside/Show/Edit/useDraft'
import { SCREEN_SCROLLER } from '../chrome'
import Meta from '../Detail/Meta'
import NavRow from './NavRow'
import { useHeading } from './useHeading'

interface Props {
  /** The entry being edited. Absent while creating: nothing is saved yet. */
  entry?: EntryMeta
  /** The kind being written: a new entry's chosen kind, or the entry's own. */
  type: EntryType
  /** The decrypted entry the draft is seeded from, or null for a new one. */
  revealed: Entry | null
}

/**
 * Writing one entry, on a phone: the nav row's two answers over a single
 * scroller holding the same title, fields and footer the desktop editor does.
 *
 * There is no accent frame around it. The desktop needs one because its editor
 * appears in the pane the read view was just in; here the screen is the mode —
 * it rose from the bottom edge and says Cancel/Save at the top.
 */
export default function Editor({ entry, type, revealed }: Props) {
  const draft = useDraft(type, revealed)
  const kind = kindOf(type)
  const heading = useHeading(type, entry)

  return (
    <div className="flex min-h-0 flex-1 flex-col animate-rise bg-screen text-text">
      <NavRow draft={draft} title={heading} />

      {/* One scroller, declared a container so the rows fold themselves (see
          fields/Row). One is also all it takes to keep a focused field visible:
          the shell around it is already sized to the visual viewport, so the
          keyboard shortens this box rather than covering it. */}
      <div
        className={`@container ${SCREEN_SCROLLER} px-4 pt-1 pb-[calc(env(safe-area-inset-bottom)+40px)]`}
      >
        {/* 56px of indent under the message: the 44px tile plus the row's gap.
            The read screen's header, input for title: the same line under it
            where the kind has something to say (a document's type, a key's
            environment), read off the draft as it is typed, so the header
            keeps its height between the two faces. */}
        <Title
          draft={draft}
          kind={kind}
          tile="h-11 w-11"
          glyph={22}
          className="flex items-center gap-3"
          message="pl-14"
        >
          {kind.eyebrow && (
            <Eyebrow entry={{ type }} revealed={draft.model} bare className="mt-0.5 truncate" />
          )}
        </Title>

        {/* The read screen's footer, so a tag is picked and drawn the same way
            in both faces. */}
        <Body draft={draft} type={type} revealed={revealed} Footer={Meta} />
      </div>
    </div>
  )
}
