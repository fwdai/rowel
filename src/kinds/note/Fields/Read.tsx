import { useTranslation } from 'react-i18next'
import SealedBlock from '@/components/elements/SealedBlock'
import { useField } from '@/components/elements/fields'

/**
 * The note body as a sealed block — the read view's. The body is the entry's
 * one secret (see meta.ts), so it is the same object an SSH private key is
 * (see `elements/SealedBlock`): sealed until asked, with what it weighs in the
 * head. The editor keeps its page (see index).
 */
export default function Read() {
  const { t } = useTranslation()
  const { value } = useField('note')
  // Code points, not UTF-16 units: what a person would count.
  const size = [...value].length

  return (
    <SealedBlock
      label="Note"
      name="note"
      value={value}
      meta={t('{{size}} chars', { size })}
      lines={[value]}
    />
  )
}
