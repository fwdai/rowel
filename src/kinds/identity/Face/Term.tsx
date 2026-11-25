import DocCell, { type Doc } from './DocCell'
import Status from './Status'
import { elapsed, validityInk } from './validity'

/**
 * A permit's term, drawn: the day it was issued at one end, the day it runs
 * out at the other, and how far along today is between them. A permit is
 * permission for a period, so the period is the thing it shows — the two dates
 * are the same cells every document has, set at the ends of their own line.
 * On a narrow card the status folds under the line rather than off its edge.
 */
export default function Term({ doc }: { doc: Doc }) {
  const expires = doc.value('expiry_date')
  const share = elapsed(doc.value('issue_date'), expires)

  return (
    <div className="flex flex-wrap items-end justify-end gap-x-3 gap-y-2">
      <DocCell doc={doc} name="issue_date" size={11} className="flex-none" />
      <div aria-hidden className="mb-2 h-[3px] min-w-10 flex-1 rounded-full bg-(--face-rule)">
        {share !== null && (
          <div
            data-testid="identity-term"
            className="relative h-full rounded-full"
            style={{ width: `${share * 100}%`, backgroundColor: validityInk(expires) ?? undefined }}
          >
            <span className="absolute -right-1 -top-[2.5px] h-2 w-2 rounded-full bg-inherit ring-2 ring-white" />
          </div>
        )}
      </div>
      <DocCell doc={doc} name="expiry_date" size={11} className="flex-none" />
      <Status iso={expires} />
    </div>
  )
}
