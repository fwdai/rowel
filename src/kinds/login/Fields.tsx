import Panel from '@/components/elements/Panel'
import {
  CustomFieldsField,
  EmailField,
  NoteField,
  OtpField,
  PasskeysField,
  PasswordField,
  UrlField,
  UsernameField,
  useFields
} from '@/components/elements/fields'
import { hasPasskey } from './meta'

// One card in both modes, the rows in one order: the credentials, the code
// that goes with the password, the rest — then the user's own rows as the
// card's last band. Switching modes never moves a row.
export default function Fields() {
  const { entry } = useFields()
  // A passkey is a credential in its own right, so it lifts the password's
  // "Required" — read through `isValid`'s own test, or the row would complain
  // in red about a draft the save then lets through.
  const passkeys = hasPasskey(entry)

  return (
    <>
      <Panel>
        <UrlField />
        <UsernameField required />
        <PasswordField required={!passkeys} />
        <OtpField />
        <EmailField />
        <NoteField label="Note" />
        <CustomFieldsField />
      </Panel>
      <PasskeysField />
    </>
  )
}
