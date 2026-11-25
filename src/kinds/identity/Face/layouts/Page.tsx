import Eye from '@/components/elements/Face/Eye'
import { LABEL_TYPE } from '@/components/elements/tokens'
import DocCell from '../DocCell'
import Holder from '../Holder'
import MachineZone from '../MachineZone'
import Status from '../Status'
import TypeLabel from '../TypeLabel'
import { placeOf } from '../place'
import type { LayoutProps } from '.'

/**
 * A passport's data page: what it is across the head, the holder beside the
 * portrait with everything the page prints about them, its own number along
 * the foot with how long it has left, and the machine-readable band under all
 * of it.
 *
 * Also the page for a document that is no particular card (`other`): which rows
 * appear is the template's business and a cell with nothing in it prints
 * nothing, so it reads as a plain sheet with no band.
 */
export default function Page({ docType, doc }: LayoutProps) {
  const place = placeOf(doc)

  return (
    <>
      <div className="relative flex min-w-0 flex-1 flex-col px-5 pb-3 pt-3">
        <header className="flex flex-none items-center justify-between gap-3 text-(--face-ink2)">
          <span className={`truncate ${LABEL_TYPE}`}>
            <TypeLabel docType={docType} />
            {place && ` · ${place}`}
          </span>
          <DocCell
            doc={doc}
            name="country"
            bare
            ink={`${LABEL_TYPE} font-medium text-(--face-ink)`}
            size={4}
            className="flex-none"
          />
        </header>

        <Holder doc={doc}>
          <DocCell doc={doc} name="nationality" size={5} />
          <DocCell doc={doc} name="birth_date" size={11} />
          <DocCell doc={doc} name="sex" size={3} />
          <DocCell doc={doc} name="issue_date" size={11} />
          <DocCell doc={doc} name="expiry_date" size={11} />
          <DocCell doc={doc} name="authority" size={16} />
          <DocCell doc={doc} name="personal_number" size={14} />
        </Holder>

        {/* The number the document is known by, set large along the foot the
            way a document prints it, with its validity in the margin. `mt-auto`
            puts every bit of the document's spare height above this line, so the
            number sits on the foot rather than floating under the fields. */}
        <div className="mt-auto flex flex-none items-end gap-2.5 pt-3">
          <DocCell doc={doc} name="number" wrap className="min-w-0 flex-1" />
          <Status iso={doc.value('expiry_date')} />
          <Eye shown={doc.shown} onToggle={doc.toggle} testid="reveal-number" className="mb-0.5" />
        </div>
      </div>

      <MachineZone docType={docType} doc={doc} />
    </>
  )
}
