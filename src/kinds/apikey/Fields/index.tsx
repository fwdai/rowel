import Panel from '@/components/elements/Panel'
import {
  CustomFieldsField,
  NoteField,
  UrlField,
  hasExtra,
  useFields
} from '@/components/elements/fields'
import { filled } from '@/components/elements/fields/formats'
import Face from '../Face'
import Scopes from './Scopes'

/**
 * An API key as an object, in both modes: the key card (see `Face`) across the
 * sheet, with the token and where it is used, then the rows a card has no room
 * for — the base URL in full, with somewhere to open it, and the scopes as
 * chips — and the note and the user's own rows in a panel of their own.
 * Editing fills the same places: reading, a panel with nothing in it is left
 * out.
 */
export default function Fields() {
  const { entry, set } = useFields()
  const editing = !!set
  const rows = editing || filled(entry.baseUrl) || filled(entry.scopes)
  const more = editing || filled(entry.note) || hasExtra(entry)

  return (
    <div className="grid gap-3">
      <Face />
      {rows && (
        <Panel>
          <UrlField name="baseUrl" label="Base URL" />
          <Scopes />
        </Panel>
      )}
      {more && (
        <Panel>
          <NoteField label="Note" />
          {/* What the fixed rows have no place for — an import's `staging`
              environment, an account id — closes the card as rows of its own. */}
          <CustomFieldsField />
        </Panel>
      )}
    </div>
  )
}
