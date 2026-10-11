import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import AddAction from '@/components/elements/AddAction'
import { PaperclipGlyph } from '@/components/Main/icons'
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
 * The way to attach a file: a quiet verb under the list, a paperclip before
 * it, in the accent — the same line "Add tag" is, so an entry with nothing
 * attached shows nothing heavier than a sentence, and one with files shows
 * the same sentence under them.
 *
 * On desktop it goes straight to the file dialog: the photos there are files
 * like any other, and the dialog's own sidebar reaches the library. A phone
 * keeps its photos out of the file browser, so there the press first asks
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
        glyph={<PaperclipGlyph size={14} />}
        testid="attach-file-button"
        onClick={() => (isMobile ? setAsking(true) : onPick('attachment'))}
        className={className}
      />
      {asking && <SourceSheet onPick={choose} onClose={() => setAsking(false)} />}
    </>
  )
}
