import Panel from '@/components/elements/Panel'
import { CustomFieldsField, Field, NoteField, useFields } from '@/components/elements/fields'
import PrivateKey from './PrivateKey'
import Fingerprint from './Fingerprint'
import Read from './Read'

export default function Fields() {
  const { set } = useFields()

  return (
    <>
      {/* Reading, the key is an object (see `Read`); the editor keeps its rows. */}
      {set ? (
        <Panel>
          <PrivateKey />
          {/* The public half is not a secret, so it is a plain copyable line. */}
          <Field name="publicKey" label="Public key" placeholder="ssh-ed25519 AAAA…" />
          <Fingerprint />
          {/* Whatever the key was protected with elsewhere — we do not encrypt it
              ourselves, so this is somewhere to keep it, not something we apply. */}
          <Field
            name="passphrase"
            label="Passphrase"
            secure
            maxLength={200}
            placeholder="••••••••"
          />
          <NoteField label="Note" />
        </Panel>
      ) : (
        <Read />
      )}
      <CustomFieldsField />
    </>
  )
}
