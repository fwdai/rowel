import Panel from '@/components/elements/Panel'
import {
  CustomFieldsField,
  DateField,
  Field,
  NoteField,
  UrlField,
  useFields
} from '@/components/elements/fields'
import Environment from './Environment'
import Read from './Read'
import Scopes from './Scopes'

export default function Fields() {
  const { set } = useFields()

  // Reading, the key is an object (see `Read`); the editor keeps its rows.
  if (!set) return <Read />

  return (
    <>
      <Panel>
        {/* Pasted rather than typed, so the row starts revealed like every
            other secret's editor and the eye takes it back. */}
        <Field name="apiKey" label="Secret key" required secure placeholder="sk_live_…" />
        <UrlField name="baseUrl" label="Base URL" />
        <Environment />
        <Scopes />
        {/* Not every key lapses; one that does says how long it has left when read. */}
        <DateField name="expiry_date" label="Expires" expiry />
        <NoteField label="Note" />
        {/* What the fixed rows have no place for — an import's `staging`
            environment, an account id — closes the card as rows of its own. */}
        <CustomFieldsField />
      </Panel>
    </>
  )
}
