import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import AddAction from '@/components/elements/AddAction'
import { isMobile } from '@/lib/platform'
import SourceSheet from './SourceSheet'
import type { AttachSource } from './useAttachments'

interface Props {
  busy: boolean
  /** Open the picker for `from` and attach what it picks. */
  onPick: (from: AttachSource) => void
  className?: string
}

/**
 * The "Attach file" action under an entry's files.
 *
 * On desktop it goes straight to the file dialog: the photos there are files
 * like any other, and the dialog's own sidebar reaches the library. A phone
 * keeps its photos out of the file browser, so there the action first asks
 * where from (`SourceSheet`), and each answer opens the picker the OS has for
 * it.
 */
export default function AttachButton({ busy, onPick, className }: Props) {
  const { t } = useTranslation()
  const [asking, setAsking] = useState(false)

  const choose = (from: AttachSource) => {
    setAsking(false)
    onPick(from)
  }

  return (
    <>
      <AddAction
        label={busy ? t('Attaching…') : t('Attach file')}
        testid="attach-file-button"
        onClick={() => (isMobile ? setAsking(true) : onPick('attachment'))}
        className={className}
      />
      {asking && <SourceSheet onPick={choose} onClose={() => setAsking(false)} />}
    </>
  )
}
