import { useTranslation } from 'react-i18next'
import SealedBlock from '@/components/elements/SealedBlock'
import { useField } from '@/components/elements/fields'
import { privateKeyFormat } from '../keyInfo'

/**
 * The private key as a sealed block — the read view's (see
 * `elements/SealedBlock`, the shape a secure note's body takes too). Its armor
 * (the BEGIN and END lines) stays legible with the format and weight in the
 * head, and the body is sealed until the eye, or the seal itself, is pressed.
 * The editor keeps its textarea row (see PrivateKey).
 */
export default function PrivateBlock() {
  const { t } = useTranslation()
  const { value } = useField('privateKey')

  if (value === '') return null

  const lines = value.trim().split('\n')
  const armored = lines.length >= 3 && lines[0].startsWith('-----BEGIN')
  // Bytes as the file would weigh, not code units: a pasted key can carry a
  // non-ASCII comment.
  const size = new TextEncoder().encode(value).length
  const meta = [privateKeyFormat(value), t('{{size}} bytes', { size })]
    .filter(Boolean)
    .join(' · ')

  return (
    // Grows: beside a plate taller than the rows, the block takes the
    // difference and its sealed body fills the panel (see `Read`).
    <SealedBlock
      label="Private key"
      name="privateKey"
      value={value}
      meta={meta}
      mono
      head={armored ? lines[0] : null}
      foot={armored ? lines[lines.length - 1] : null}
      lines={armored ? lines.slice(1, -1) : lines}
      className="flex-1"
    />
  )
}
