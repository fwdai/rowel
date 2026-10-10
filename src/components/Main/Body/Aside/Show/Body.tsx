import type { ComponentType } from 'react'
import { editEntry } from '@/store'
import type { Entry, EntryMeta } from '@/api/types'
import { FieldsProvider } from '@/components/elements/fields'
import { kindOf } from '@/kinds'
import Attachments from './Attachments'
import { takesAttachments } from './Attachments/drop'
import DeleteError from './DeleteError'
import DefaultFooter, { type FooterProps } from './Footer'

interface Props {
  entry: EntryMeta
  /** The decrypted entry, or null while `revealEntry` is still in flight. */
  revealed: Entry | null
  /** What the last delete had to say, from `useDelete`. */
  error: string | null
  /**
   * What draws the entry's tags and dates under the rows. The desktop's strip
   * by default; the phone hands in its own (`Compact/Detail/Meta`), which lays
   * the same facts out as sections and a meta line. The facts are read here
   * either way, so the two can never disagree about what the entry is filed
   * under or when it was touched.
   */
  Footer?: ComponentType<FooterProps>
}

// Everything under the identity header while reading: the kind's own field set
// in its read face, the entry's attached files, the footer, and whatever the
// last delete had to say. The mirror of `Edit/Body` — a shell chooses what the
// header above it looks like, where the whole thing scrolls, and how the
// footer's facts are laid out, and adds nothing of its own.
export default function Body({ entry, revealed, error, Footer = DefaultFooter }: Props) {
  const Fields = kindOf(entry.type).Fields

  return (
    <>
      {revealed && (
        <div className="mt-5">
          {/* No writer: every field in the set renders its read face. */}
          <FieldsProvider value={{ entry: { ...revealed }, set: null, attempted: false }}>
            <Fields />
          </FieldsProvider>
          {/* Keyed: each entry's files are its own, read when it opens. */}
          {!entry.deletedAt && takesAttachments(entry.type) && (
            <Attachments key={entry.id} entry={entry} />
          )}
        </div>
      )}

      {/* Tags are metadata, so the footer needs no reveal to render. */}
      <Footer
        tags={entry.tags}
        onAdd={entry.deletedAt ? undefined : () => editEntry()}
        createdAt={entry.createdAt}
        updatedAt={entry.updatedAt}
        deletedAt={entry.deletedAt}
      />

      <DeleteError error={error} />
    </>
  )
}
