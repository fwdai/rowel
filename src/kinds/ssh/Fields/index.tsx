import { useTranslation } from 'react-i18next'
import Panel from '@/components/elements/Panel'
import {
  CopyValue,
  CustomFieldsField,
  Field,
  FieldRow,
  NoteField,
  hasExtra,
  useFields
} from '@/components/elements/fields'
import { filled } from '@/components/elements/fields/formats'
import { VALUE } from '@/components/elements/tokens'
import { cx } from '@/utils/cx'
import Face from '../Face'
import { parsePublicKey } from '../keyInfo'
import PrivateBlock from './PrivateBlock'

/**
 * An SSH key as an object, in both modes: the fingerprint plate (see `Face`)
 * beside the public half — the line, the comment it ends in, the passphrase
 * kept with it — and the private key sealed under them. The note and the
 * user's own rows take a panel of their own under the lot.
 *
 * Editing changes no part's place: the public line and the passphrase are
 * typed into their rows, the private key onto its block, and the plate and
 * the comment — both derived from the key — are printed as they are read. A
 * key the generator writes puts its plate up as it lands.
 */
export default function Fields() {
  const { t } = useTranslation()
  const { entry, set } = useFields()
  const editing = !!set
  const publicKey = filled(entry.publicKey) ? entry.publicKey : ''
  const { comment } = parsePublicKey(publicKey)
  const plate = filled(entry.fingerprint)
  const rows = editing || publicKey !== '' || filled(entry.passphrase)
  const more = editing || filled(entry.note) || hasExtra(entry)

  return (
    <div className="grid gap-3">
      {/* The plate keeps its column at every pane width — 250px on the full
          sheet, as the prototype has it; on a narrower pane it gives way to
          the rows down to the 160px the randomart needs (see `Face`, which
          tightens itself to fit). The two columns stretch to one height: the
          taller sets it, the plate spreads out to match, and on the other side
          the sealed block grows to fill. The phone is the one place the two
          stack: beside 44px rails a 160px plate left the rows a sliver and
          stretched itself to the sealed block's height, so there the plate is
          the screen's first card, like every other kind's face. */}
      <div
        className={cx(
          'grid gap-3',
          plate && 'md:grid-cols-[clamp(160px,30%,250px)_minmax(0,1fr)]'
        )}
      >
        {plate && <Face />}
        <div className="flex flex-col gap-3">
          {rows && (
            <Panel>
              {/* The public half is not a secret, so it is a plain line. */}
              <Field name="publicKey" label="Public key" placeholder="ssh-ed25519 AAAA…" />
              {comment && (
                <FieldRow label="Comment">
                  {() => (
                    <CopyValue
                      value={comment}
                      label={`${t('Comment')} · ${t('Copy')}`}
                      testid="entry-value-comment"
                      className={`${VALUE} text-text`}
                    >
                      {comment}
                    </CopyValue>
                  )}
                </FieldRow>
              )}
              {/* Whatever the key was protected with elsewhere — we do not
                  encrypt it ourselves, so this is somewhere to keep it, not
                  something we apply. */}
              <Field
                name="passphrase"
                label="Passphrase"
                secure
                maxLength={200}
                placeholder="••••••••"
              />
            </Panel>
          )}
          <PrivateBlock />
        </div>
      </div>
      {more && (
        <Panel>
          <NoteField label="Note" />
          <CustomFieldsField />
        </Panel>
      )}
    </div>
  )
}
