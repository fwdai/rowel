import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import AddAction from '@/components/elements/AddAction'
import SourceSheet from '@/components/elements/SourceSheet'
import { isMobile } from '@/lib/platform'
import type { AttachSource } from './useAttachments'

interface Props {
  busy: boolean
  /** Open the picker for `from` and attach what it picks; whether it picked. */
  onPick: (from: AttachSource) => Promise<boolean>
  /** Attach a file the webview holds — a photo the camera just took. */
  onFile: (file: File) => void
  className?: string
}

/**
 * The "Attach file" action under an entry's files.
 *
 * On desktop it goes straight to the file dialog: the photos there are files
 * like any other, and the dialog's own sidebar reaches the library. A phone
 * keeps its photos out of the file browser, so there the action first asks
 * where from (`SourceSheet`): the camera, the photo library or Files, each
 * answer opening the picker the OS has for it. A picker backed out of leaves
 * the question open; a photo taken or a file picked answers it.
 */
export default function AttachButton({ busy, onPick, onFile, className }: Props) {
  const { t } = useTranslation()
  const [asking, setAsking] = useState(false)

  const choose = async (from: AttachSource) => {
    if (await onPick(from)) setAsking(false)
  }

  const captured = (file: File) => {
    setAsking(false)
    onFile(file)
  }

  return (
    <>
      <AddAction
        label={busy ? t('Attaching…') : t('Attach file')}
        testid="attach-file-button"
        onClick={() => (isMobile ? setAsking(true) : void onPick('attachment'))}
        className={className}
      />
      {asking && (
        <SourceSheet
          testid="attach-source"
          title={t('Attach file')}
          description={t('Everything is encrypted before it touches disk.')}
          onClose={() => setAsking(false)}
          onCapture={captured}
          onLibrary={() => void choose('photo')}
          onBrowse={() => void choose('attachment')}
        />
      )}
    </>
  )
}
