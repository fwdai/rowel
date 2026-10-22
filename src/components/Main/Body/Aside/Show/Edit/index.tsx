import type { Entry, EntryType } from '@/api/types'
import { kindOf } from '@/kinds'
import { useTranslation } from 'react-i18next'
import { LABEL } from '@/components/elements/tokens'
import Actions from './Actions'
import Body from './Body'
import Title from './Title'
import { useDraft } from './useDraft'

interface Props {
  /** The kind being written: a new entry's chosen kind, or the entry's own. */
  type: EntryType
  /** The decrypted entry, or null for a new one (and while the reveal is in flight). */
  revealed: Entry | null
}

// Editing happens in the detail pane, in the read view's own layout: same
// eyebrow, same title line, same rows in the same places. What changes is the
// state — an accent frame around the pane, an accent eyebrow, and a title you
// can type in — so there is never a doubt about which mode this is.
//
// The parts below the frame are the phone form's too (Compact/Form): this file
// is only where the desktop puts them.
export default function Edit({ type, revealed }: Props) {
  const { t } = useTranslation()
  const draft = useDraft(type, revealed)
  const kind = kindOf(type)

  return (
    // A container, like the read view's root: the editor renders the same rows
    // through the same geometry, so it folds where they fold.
    <div className="@container mx-auto w-full max-w-sheet">
      {/* Negative margin cancels the frame's padding, and the frame is an inset
          ring rather than a border, so the content sits exactly where the read
          view puts it — not a pixel in or down. */}
      <div
        data-testid="entry-sheet"
        className="-m-4 rounded-xl p-4 ring-1 ring-accent-line ring-inset"
      >
        {/* The eyebrow shares its line with the actions, so the title input below
            runs the full content width and its underline ends where the rows do.
            The read eyebrow's shape — the kind, then what the kind has to say —
            with "Editing" in the accent where the host was, so the kind's name
            stays put and only the second segment changes. */}
        <div className="flex items-center justify-between gap-4">
          <div
            className={`flex min-w-0 flex-1 items-center gap-2 truncate whitespace-nowrap ${LABEL}`}
          >
            <span>{t(kind.label)}</span>
            <span>·</span>
            <span className="text-accent">{t('Editing')}</span>
          </div>
          <Actions draft={draft} />
        </div>

        {/* 38px of indent under the message: the 28px tile plus the row's gap. */}
        <Title
          draft={draft}
          kind={kind}
          tile="h-7 w-7"
          glyph={16}
          className="mt-2 flex items-center gap-2.5"
          message="pl-[38px]"
        />

        <Body draft={draft} type={type} revealed={revealed} />
      </div>
    </div>
  )
}
