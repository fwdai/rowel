import type { ReactNode } from 'react'
import DocCell, { type Doc } from './DocCell'
import Portrait from './Portrait'

// The printed fields, flowing beside the portrait and on under it — each as wide
// as its own caption needs and no wider, the way a document sets them. Not
// columns: a fixed column count is a decision about the page that the page has
// not asked for, and on a narrow pane it is the thing that starts dictating how
// tall the document has to be.
//
// The flow sizes itself and the data area takes its height from that — so a pane
// too narrow to set these fields pushes the face taller rather than hiding the
// overflow under the frame's rounded corner. It must
// not be handed a height to fit into — a wrapping flex container asked to fit a
// box shorter than its lines lays them out anyway and spills over whatever is
// below, and its intrinsic height does not travel back up through a `flex-1`
// parent to stop that happening. So nothing above it stretches, and the slack on
// a document with room to spare is collected above its foot instead.
const FIELDS = 'flex min-w-0 flex-1 flex-wrap content-start gap-x-5 gap-y-2.5'

interface Props {
  doc: Doc
  /** The name's number on a driving licence (see `DocCell`). */
  index?: string
  /**
   * The cells that follow the name. Editing, a cell is as wide as its box; the
   * sizes are about what each value runs to, so the flow stays the document's.
   */
  children: ReactNode
}

/**
 * The holder, as every document shows them: the photograph, and beside it
 * their name — the biggest thing a document prints about them — over whatever
 * else it says, in the flow.
 */
export default function Holder({ doc, index, children }: Props) {
  return (
    <div className="mt-3 flex items-start gap-4">
      <Portrait src={doc.portrait} onRemove={doc.removePortrait} />
      <div className={FIELDS}>
        <DocCell
          doc={doc}
          name="name"
          label="Holder"
          index={index}
          ink="text-lg font-medium uppercase tracking-[0.03em]"
          wrap
          className="w-full"
        />
        {children}
      </div>
    </div>
  )
}
