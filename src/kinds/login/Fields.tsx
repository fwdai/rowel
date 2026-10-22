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
import { filled } from '@/components/elements/fields/formats'
import { BAND_GUTTER } from '@/components/elements/tokens'
import { hasPasskey } from './meta'

// One card in both modes, cut into bands wherever what the rows are for
// changes, the way a grouped list is: the credentials first — the name, the
// password and the code that goes with it, what signing in takes — then where
// the account lives and what is worth knowing about it, then the user's own
// rows as the card's last band. The same order in both modes, so switching
// never moves a row. A login's identity — the favicon, the host — is the
// header overhead already, so the card starts on the credentials rather than
// saying the address again first.
export default function Fields() {
  const { entry, set } = useFields()
  // A passkey is a credential in its own right, so it lifts the password's
  // "Required" — read through `isValid`'s own test, or the row would complain
  // in red about a draft the save then lets through.
  const passkeys = hasPasskey(entry)
  // Reading, an empty row is no row; a band of nothing would be a gutter
  // under the credentials with nothing after it.
  const about = !!set || filled(entry.website) || filled(entry.email) || filled(entry.note)

  return (
    <>
      <Panel>
        {/* Each band is its own row scope, so the hairline stops at the band's
            last row (ROW_HAIRLINE drops it on `last`). */}
        <div>
          <UsernameField required />
          <PasswordField required={!passkeys} />
          <OtpField />
        </div>
        {about && (
          <>
            <div data-gutter className={BAND_GUTTER} />
            <div>
              <UrlField />
              <EmailField />
              <NoteField label="Note" />
            </div>
          </>
        )}
        <CustomFieldsField />
      </Panel>
      <PasskeysField />
    </>
  )
}
