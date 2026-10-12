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
 * An API key read as an object: the key card (see `Face`) across the sheet,
 * with the token and where it is used, then the rows a card has no room for —
 * the base URL in full, with somewhere to open it, and the scopes as chips —
 * and the note and the user's own rows, when there are any, in a panel of
 * their own.
 */
export default function Read() {
  const { entry } = useFields()
  const rows = filled(entry.baseUrl) || filled(entry.scopes)

  return (
    <div className="grid gap-3">
      <Face />
      {rows && (
        <Panel>
          <UrlField name="baseUrl" label="Base URL" />
          <Scopes />
        </Panel>
      )}
      {(filled(entry.note) || hasExtra(entry)) && (
        <Panel>
          <NoteField label="Note" />
          <CustomFieldsField />
        </Panel>
      )}
    </div>
  )
}
