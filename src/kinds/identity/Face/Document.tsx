import { useTranslation } from 'react-i18next'
import Eye from '@/components/elements/Face/Eye'
import { LABEL_TYPE } from '@/components/elements/tokens'
import { countryName } from '@/utils/countries'
import { daysUntil, relativeFuture } from '@/utils/time'
import { DOC_TYPE_LABELS, type DocType } from '../templates'
import DocCell, { type Doc } from './DocCell'
import Frame from './Frame'
import Portrait from './Portrait'
import { mrz } from './mrz'

interface Props {
  docType: DocType
  doc: Doc
}

/*
 * What each document measures, in millimetres, as ISO/IEC 7810 and ICAO 9303
 * have it. A passport's data page is ID-3 and noticeably squarer than the ID-1
 * card a licence, an ID card and a permit all share — which is the credit card's
 * own shape, because a credit card is an ID-1 card.
 *
 * These are the fixed dimension of a face: the document is this shape, and the
 * printing is arranged to fit inside it. `other` has no physical form to be
 * honest about, so it takes the wallet-sized default.
 */
const FORMAT: Record<DocType, string> = {
  passport: '125 / 88',
  id_card: '85.6 / 53.98',
  driver_license: '85.6 / 53.98',
  residence_permit: '85.6 / 53.98',
  other: '85.6 / 53.98'
}

// The printed fields, flowing beside the portrait and on under it — each as wide
// as its own caption needs and no wider, the way a document sets them. Not
// columns: a fixed column count is a decision about the page that the page has
// not asked for, and on a narrow pane it is the thing that starts dictating how
// tall the document has to be.
//
// The flow sizes itself and the data area takes its height from that. It must
// not be handed a height to fit into — a wrapping flex container asked to fit a
// box shorter than its lines lays them out anyway and spills over whatever is
// below, and its intrinsic height does not travel back up through a `flex-1`
// parent to stop that happening. So nothing above it stretches, and the slack on
// a document with room to spare is collected above the number instead.
const FIELDS = 'flex min-w-0 flex-1 flex-wrap content-start gap-x-5 gap-y-2.5'

// The strip along the bottom edge, running the full width of the paper the way
// it is printed — under the printed area rather than inside its margins.
const BAND = 'border-t border-(--face-rule) bg-(--face-tail) px-5 py-2'

// The status pill's three tones. Fixed hex like the rest of the face: the paper
// stays light when the app goes dark, so `text-bad` would invert underneath it.
const EXPIRED = '#B3261E'
const SOON = '#8A5A00'
const VALID = '#1E7A55'

// Half a year out is where "expires" starts to mean something: most countries
// want six months left on a document before they will let you travel on it.
const SOON_DAYS = 180

/*
 * Whether the document is still good, said once and where a document says it —
 * beside the number, in the margin. Every date cell used to carry its own
 * countdown, which stated the same fact twice on any document that printed both
 * an issue and an expiry date. A document with no expiry says nothing.
 */
function Status({ iso }: { iso: string }) {
  const { t } = useTranslation()
  const days = daysUntil(iso)
  if (days === null) return null

  const ink = days < 0 ? EXPIRED : days <= SOON_DAYS ? SOON : VALID

  return (
    <span
      style={{ color: ink, borderColor: ink }}
      className={`flex h-5 flex-none items-center gap-1.5 rounded-xs border px-1.5 ${LABEL_TYPE}`}
    >
      <span aria-hidden className="h-1 w-1 rounded-full bg-current" />
      {days < 0 ? t('Expired') : relativeFuture(iso)}
    </span>
  )
}

// The bars of a driving licence's barcode. Decoration, like the portrait's
// silhouette: the vault holds no barcode, and what the code would carry is
// printed above it in full.
const BARS =
  'repeating-linear-gradient(90deg, #1B1D21 0 2px, transparent 2px 4px, #1B1D21 4px 5px, transparent 5px 9px, #1B1D21 9px 12px, transparent 12px 14px)'

/*
 * What a machine reads off the document.
 *
 * The MRZ a passport, an ID card and a residence permit all carry, generated
 * from the fields themselves — check digits and all, see `mrz.ts` — and the
 * barcode a driving licence has in its place. This is the one thing on the face
 * set in the mono tier, because it is the one thing on it that is machine text
 * rather than something a person reads.
 */
function MachineZone({ docType, doc }: Props) {
  const lines = mrz(docType, doc)

  if (lines)
    return (
      <div className={`${BAND} overflow-hidden`}>
        {lines.map((line, row) => (
          <div
            key={row}
            className="whitespace-pre font-mono text-2xs leading-[1.6] tracking-[0.1em]"
          >
            {line}
          </div>
        ))}
      </div>
    )

  if (docType !== 'driver_license') return null

  return (
    <div className={BAND}>
      <div aria-hidden className="h-6 opacity-55" style={{ backgroundImage: BARS }} />
    </div>
  )
}

/**
 * An identity document as an object: what it is across the head, the holder
 * beside the portrait with everything the document prints about them, its own
 * number along the foot with how long it has left, and the machine-readable
 * band under all of it.
 *
 * One layout for every type. Which rows appear is the template's business and
 * a cell with nothing in it prints nothing, so a passport and a licence differ
 * here only in their proportions and in what a machine reads off the bottom.
 */
export default function Document({ docType, doc }: Props) {
  const { t } = useTranslation()
  const country = doc.value('country')
  const place = countryName(country) ?? country

  return (
    <Frame ratio={FORMAT[docType]} docType={docType}>
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
            <DocCell
              doc={doc}
              name="name"
              label="Holder"
              ink="text-lg font-medium uppercase tracking-[0.03em]"
              wrap
              className="w-full"
            />
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
    </Frame>
  )
}
