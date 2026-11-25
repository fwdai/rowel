import { useTranslation } from 'react-i18next'
import Eye from '@/components/elements/Face/Eye'
import { LABEL_TYPE } from '@/components/elements/tokens'
import { countryName } from '@/utils/countries'
import { DOC_TYPE_LABELS } from '../../templates'
import DocCell from '../DocCell'
import MachineZone from '../MachineZone'
import Portrait from '../Portrait'
import Status from '../Status'
import { FIELDS, NAME } from '../flow'
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
  const { t } = useTranslation()
  const country = doc.value('country')
  const place = countryName(country) ?? country

  return (
    <>
      <div className="relative flex min-w-0 flex-1 flex-col px-5 pb-3 pt-3">
        <header className="flex flex-none items-center justify-between gap-3 text-(--face-ink2)">
          <span className={`truncate ${LABEL_TYPE}`}>
            <span data-testid="identity-doc-type">{t(DOC_TYPE_LABELS[docType])}</span>
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

        {/* The data area, as tall as the printing in it — never told a height,
            so a pane too narrow to set these fields pushes the face taller
            rather than hiding the overflow under the frame's rounded corner. */}
        <div className="mt-3 flex items-start gap-4">
          <Portrait />
          <div className={FIELDS}>
            <DocCell doc={doc} name="name" label="Holder" ink={NAME} wrap className="w-full" />
            {/* Editing, a cell is as wide as its box; the sizes are about what
                each value runs to, so the flow stays the document's. */}
            <DocCell doc={doc} name="nationality" size={5} />
            <DocCell doc={doc} name="birth_date" size={11} />
            <DocCell doc={doc} name="sex" size={3} />
            <DocCell doc={doc} name="issue_date" size={11} />
            <DocCell doc={doc} name="expiry_date" size={11} />
            <DocCell doc={doc} name="authority" size={16} />
            <DocCell doc={doc} name="personal_number" size={14} />
          </div>
        </div>

        {/* The number the document is known by, set large along the foot the
            way a document prints it, with its validity in the margin. `mt-auto`
            puts every bit of the document's spare height above this line, so the
            number sits on the foot rather than floating under the fields. */}
        <div className="mt-auto flex flex-none items-end gap-2.5 pt-3">
          <DocCell
            doc={doc}
            name="number"
            ink="text-xl tracking-secret"
            wrap
            className="min-w-0 flex-1"
          />
          <Status iso={doc.value('expiry_date')} />
          <Eye shown={doc.shown} onToggle={doc.toggle} testid="reveal-number" className="mb-0.5" />
        </div>
      </div>

      <MachineZone docType={docType} doc={doc} />
    </>
  )
}
