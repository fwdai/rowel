import { Fragment } from 'react'
import { useTranslation } from 'react-i18next'
import Panel from '@/components/elements/Panel'
import {
  CustomFieldsField,
  DateField,
  Field,
  NoteField,
  hasExtra,
  useFields
} from '@/components/elements/fields'
import { filled } from '@/components/elements/fields/formats'
import { BAND_GUTTER, FACE_ASIDE, LABEL } from '@/components/elements/tokens'
import { countryName } from '@/utils/countries'
import { cx } from '@/utils/cx'
import { docTypeOf, specOf, DOC_TYPE_LABELS, TEMPLATES, type Row } from '../templates'
import { DOC_GLYPHS } from '../glyphs'
import Face from '../Face'

// The template's rows cut into bands wherever the group changes — holder,
// document, extras. The order is the template's business, so this only has to
// watch for the change.
const bands = (rows: Row[]): Row[][] =>
  rows.reduce<Row[][]>((out, row) => {
    const open = out[out.length - 1]
    if (open && specOf(open[0].key).group === specOf(row.key).group) open.push(row)
    else out.push([row])
    return out
  }, [])

// An ID document, rendered from its template: the document type picks the rows,
// and each row is one of the three faces the field set already has (a text row,
// a date row, the note body). No row is written out here, so adding a field to a
// document is an entry in `templates.ts` and nothing else.
//
// Which document it is was settled before the form opened — the add picker
// asks, and the answer rides in on the draft — so the form never changes shape
// under the user: there is no switch here, only a caption saying what it is.
export default function Fields() {
  const { t } = useTranslation()
  const { entry, set } = useFields()
  const editing = !!set
  const docType = docTypeOf(entry)

  // Reading, the document is an object (see `Face`); the note it carries and
  // any custom fields keep their rows on a panel beside it, as on a credit card.
  if (!editing) {
    const aside = filled(entry.note) || hasExtra(entry)
    return (
      <div className={aside ? FACE_ASIDE : undefined}>
        <Face />
        {aside && (
          // Its own container, so its rows fold beside the face (see the card).
          <Panel className="@container">
            <NoteField label="Note" />
            <CustomFieldsField />
          </Panel>
        )}
      </div>
    )
  }

  const rows = TEMPLATES[docType]
  const DocGlyph = DOC_GLYPHS[docType]

  const row = ({ key, required }: Row) => {
    const spec = specOf(key)
    if (spec.note)
      return <NoteField key={key} name={key} label={spec.label} required={required} />
    if (spec.date)
      return (
        <DateField
          key={key}
          name={key}
          label={spec.label}
          required={required}
          expiry={spec.expiry}
        />
      )
    return (
      <Field
        key={key}
        name={key}
        label={spec.label}
        required={required}
        secure={spec.secret}
        big={spec.big}
        placeholder={spec.placeholder}
        maxLength={spec.maxLength}
        suffix={spec.country ? countryName : undefined}
      />
    )
  }

  return (
    <>
      {/* The document's name over its panel, where a grouped list captions a
          section: inset to the rows' own text edge, with the mark the picker
          showed it under, so the form says which document it is cut for. */}
      <div className={cx('mb-2 flex items-center gap-1.5 px-4', LABEL)}>
        <DocGlyph size={12} />
        <span data-testid="identity-doc-type">{t(DOC_TYPE_LABELS[docType])}</span>
      </div>
      <Panel>
        {bands(rows).map((band, index) => (
          <Fragment key={specOf(band[0].key).group}>
            {/* A gutter cut through the panel: the bands read apart without a
                heading over each of them. */}
            {index > 0 && <div data-gutter className={BAND_GUTTER} />}
            {/* Each band is its own row scope, so the hairline stops at the
                band's last row (ROW_HAIRLINE drops it on `last`). */}
            <div>{band.map(row)}</div>
          </Fragment>
        ))}
        {/* The user's own rows close the card, with the row that adds one. */}
        <CustomFieldsField />
      </Panel>
    </>
  )
}
