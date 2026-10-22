import { useTranslation } from 'react-i18next'
import { openSshGenerator } from '@/store'
import IconButton from '@/components/elements/IconButton'
import SealedBlock from '@/components/elements/SealedBlock'
import { useField, useFields } from '@/components/elements/fields'
import { requiredError } from '@/components/elements/fields/formats'
import { RefreshGlyph } from '@/components/Main/icons'
import { privateKeyFormat } from '../keyInfo'

/**
 * The private key as a sealed block in both modes (see `elements/SealedBlock`,
 * the shape a secure note's body takes too). Read, its armor (the BEGIN and
 * END lines) stays legible with the format and weight in the head, and the
 * body is sealed until the eye, or the seal itself, is pressed. Edited, the
 * same block is the page the key is pasted on, with the generator in its
 * head — a keypair is three draft keys, so Generate takes the field-set
 * writer rather than this row's own setter.
 */
export default function PrivateBlock() {
  const { t } = useTranslation()
  const { set: write, attempted } = useFields()
  const { value, set, editing } = useField('privateKey')

  if (!editing && value === '') return null

  const lines = value.trim().split('\n')
  const armored = !editing && lines.length >= 3 && lines[0].startsWith('-----BEGIN')
  // Bytes as the file would weigh, not code units: a pasted key can carry a
  // non-ASCII comment.
  const size = new TextEncoder().encode(value).length
  const meta = value
    ? [privateKeyFormat(value), t('{{size}} bytes', { size })].filter(Boolean).join(' · ')
    : undefined

  const generate = () =>
    openSshGenerator(pair => {
      write?.('privateKey', pair.privateKey)
      write?.('publicKey', pair.publicKey)
      write?.('fingerprint', pair.fingerprint)
    })

  return (
    // Grows: beside a plate taller than the rows, the block takes the
    // difference and its body fills the panel (see `Fields`).
    <SealedBlock
      label="Private key"
      name="privateKey"
      value={value}
      meta={meta}
      mono
      head={armored ? lines[0] : null}
      foot={armored ? lines[lines.length - 1] : null}
      lines={armored ? lines.slice(1, -1) : lines}
      onChange={editing ? set : undefined}
      placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
      error={editing ? requiredError(value, true, attempted) : undefined}
      actions={
        editing && (
          <IconButton title={t('Generate')} testid="generate-ssh-key-link" onClick={generate}>
            <RefreshGlyph />
          </IconButton>
        )
      }
      className="flex-1"
    />
  )
}
