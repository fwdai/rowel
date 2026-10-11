import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import ActionSheet from '@/components/elements/ActionSheet'
import AddAction from '@/components/elements/AddAction'
import { isMobile } from '@/lib/platform'
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
 * where from, as an action sheet (`ActionSheet`) — the photo library or Files,
 * under the names the system gives them — and each answer opens the system
 * picker the OS has for it, run from Rust. Not the webview's own file input,
 * as the scan is: its sheet would offer the camera too, and a file to attach
 * is one the user already has.
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
      {asking && (
        <ActionSheet
          testid="attach-source-sheet"
          title={t('Attach file')}
          onClose={() => setAsking(false)}
          actions={[
            {
              testid: 'attach-photo-button',
              label: t('Photo Library'),
              onSelect: () => choose('photo')
            },
            {
              testid: 'attach-from-files-button',
              label: t('Choose File…'),
              onSelect: () => choose('attachment')
            }
          ]}
        />
      )}
    </>
  )
}
