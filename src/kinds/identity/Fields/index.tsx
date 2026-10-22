import Panel from '@/components/elements/Panel'
import { CustomFieldsField, NoteField, hasExtra, useFields } from '@/components/elements/fields'
import { filled } from '@/components/elements/fields/formats'
import { FACE_ASIDE } from '@/components/elements/tokens'
import Face from '../Face'

// An ID document as an object, in both modes: the face (see `Face`), printed
// from its template, with the note it carries and any custom fields on a
// panel beside it, as on a credit card. Editing, the face is the form — each
// printed value an input in its place — so the document never changes shape
// between reading and writing. Which document it is was settled before the
// form opened (the add picker asks, and the answer rides in on the draft).
export default function Fields() {
  const { entry, set } = useFields()
  const aside = !!set || filled(entry.note) || hasExtra(entry)

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
