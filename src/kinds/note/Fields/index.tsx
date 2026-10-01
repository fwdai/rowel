import Panel from '@/components/elements/Panel'
import { CustomFieldsField, NoteField, useFields } from '@/components/elements/fields'
import Read from './Read'

export default function Fields() {
  const { set } = useFields()

  return (
    <>
      {set ? (
        <Panel>
          {/* No label: the body is the entry, and the title is already overhead. */}
          <NoteField required />
        </Panel>
      ) : (
        <Read />
      )}
      <CustomFieldsField />
    </>
  )
}
