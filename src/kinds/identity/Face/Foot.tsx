import type { ReactNode } from 'react'
import Eye from '@/components/elements/Face/Eye'
import DocCell, { type Doc } from './DocCell'
import Status from './Status'

interface Props {
  doc: Doc
  /** The number's own number on a driving licence (see `DocCell`). */
  index?: string
  /** What else the foot prints, between the number and the margin. */
  children?: ReactNode
}

/**
 * A document's foot: the number it is known by, set large the way a document
 * prints it, with how long it has left and the eye in the margin. `mt-auto`
 * puts every bit of the document's spare height above this line, so the number
 * sits on the foot rather than floating under the fields. On a narrow document
 * the margin folds under the number rather than squeezing it to a column of
 * single characters.
 */
export default function Foot({ doc, index, children }: Props) {
  return (
    <div className="mt-auto flex flex-none flex-wrap items-end justify-end gap-x-2.5 gap-y-2 pt-3">
      <DocCell doc={doc} name="number" index={index} wrap className="min-w-0 flex-1 basis-32" />
      {children}
      <div className="flex flex-none items-end gap-2.5">
        <Status iso={doc.value('expiry_date')} />
        <Eye shown={doc.shown} onToggle={doc.toggle} testid="reveal-number" className="mb-0.5" />
      </div>
    </div>
  )
}
