import Panel from '@/components/elements/Panel'
import { CustomFieldsField, NoteField, hasExtra, useFields } from '@/components/elements/fields'
import Read from './Read'

export default function Fields() {
  const { entry, set } = useFields()

  if (set)
    return (
      <Panel>
        {/* No label: the body is the entry, and the title is already overhead. */}
        <NoteField required />
        {/* The user's own rows close the card, with the row that adds one. */}
        <CustomFieldsField />
      </Panel>
    )

  return (
    <div className="grid gap-3">
      <Read />
      {/* The body is a sealed block (see `Read`), so the rows the user added
          take a card of their own under it, where every other kind has them
          under the note. */}
      {hasExtra(entry) && (
        <Panel>
          <CustomFieldsField />
        </Panel>
      )}
    </div>
  )
}
