import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import AddAction from '@/components/elements/AddAction'
import Panel from '@/components/elements/Panel'
import {
  CustomFieldsField,
  EmailField,
  FieldRow,
  NoteField,
  OtpField,
  PasskeysField,
  PasswordField,
  UrlField,
  UsernameField,
  useFields
} from '@/components/elements/fields'
import { hasPasskey } from './meta'

export default function Fields() {
  const { t } = useTranslation()
  const { entry, set } = useFields()
  const editing = !!set
  // The dial's column opens for a saved code, or once the editor asks for one.
  // Until then the panel keeps the read view's full width, so entering edit
  // does not reflow the rows. Sticky once open: clearing the box mid-edit must
  // not take the column away from under the caret. (Edit is keyed per entry,
  // so this never carries over to the next session.)
  const [opened, setOpened] = useState(!!entry.otp)
  const otp = editing ? opened : !!entry.otp
  // A passkey is a credential in its own right, so it lifts the password's
  // "Required" — read through `isValid`'s own test, or the row would complain
  // in red about a draft the save then lets through.
  const passkeys = hasPasskey(entry)

  return (
    <>
      <div
        className={
          // Editing, the dial's panel takes a fixed 208px column, so a
          // container that cannot spare it puts the panel back on the full
          // width and the dial under it — otherwise the rows would be read
          // through a 140px window. Reading, the code is a row of the panel
          // itself, under the password it goes with.
          editing && otp
            ? 'grid grid-cols-[minmax(0,1fr)_208px] items-start gap-3 @max-[420px]:grid-cols-1'
            : 'grid gap-3'
        }
      >
        <Panel>
          <UrlField />
          <UsernameField required />
          <PasswordField required={!passkeys} />
          {!editing && <OtpField />}
          <EmailField />
          <NoteField label="Note" />
          {editing && !otp && (
            <FieldRow label="One-time code">
              {id => (
                // The value line's height, so the row does not shift when the
                // dial's field takes its place.
                <AddAction
                  id={id}
                  label={t('Add one-time code')}
                  testid="add-otp-button"
                  onClick={() => setOpened(true)}
                  className="h-6"
                />
              )}
            </FieldRow>
          )}
        </Panel>
        {editing && otp && <OtpField autoFocus={!entry.otp} />}
      </div>
      <PasskeysField />
      <CustomFieldsField />
    </>
  )
}
