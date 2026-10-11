import { useRef } from 'react'
import { useTranslation } from 'react-i18next'
import AddAction from '@/components/elements/AddAction'
import FileInput from '@/components/elements/FileInput'
import { isMobile } from '@/lib/platform'

interface Props {
  busy: boolean
  /** Open the OS file dialog and attach what it picks (the desktop). */
  onPick: () => void
  /** Attach a file the system's picker handed the webview (the phone). */
  onFile: (file: File) => void
  className?: string
}

/**
 * The "Attach file" action under an entry's files.
 *
 * On desktop it goes straight to the file dialog: the photos there are files
 * like any other, and the dialog's own sidebar reaches the library. A phone
 * keeps its photos out of the file browser, so there the action opens the
 * system's own sheet (`FileInput`) — the photo library, the camera or Files —
 * and attaches what any of them hands back.
 */
export default function AttachButton({ busy, onPick, onFile, className }: Props) {
  const { t } = useTranslation()
  const input = useRef<HTMLInputElement>(null)

  return (
    <>
      {isMobile && <FileInput ref={input} testid="attach-file-input" onFile={onFile} />}
      <AddAction
        label={busy ? t('Attaching…') : t('Attach file')}
        testid="attach-file-button"
        onClick={() => (isMobile ? input.current?.click() : onPick())}
        className={className}
      />
    </>
  )
}
