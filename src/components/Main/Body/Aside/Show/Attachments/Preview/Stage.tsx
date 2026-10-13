import { useMemo, type MouseEvent, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { Attachment } from '@/api/types'
import Button from '@/components/elements/Button'
import { META } from '@/components/elements/tokens'
import { cx } from '@/utils/cx'
import { fileCaption, type PreviewKind } from '../kind'
import Thumb from '../Thumb'
import type { AttachmentUrl } from '../useAttachmentUrl'
import Picture from './Picture'

interface Props {
  kind: PreviewKind
  attachment: Attachment
  file: AttachmentUrl
  onSave: () => void
  /** A press on the ground beside the file, not on the file. */
  onGround: () => void
}

// A document on the desktop stands as a sheet on the dimmed ground, inset
// from the window's edges; on the phone it is the screen.
const SHEET = 'md:mx-auto md:my-6 md:w-full md:max-w-3xl md:rounded-lg md:bg-detail md:shadow-float'

/**
 * The file, as the webview can show it: a picture fitted to the stage
 * (`Picture`, which the desktop can also show at its actual size), a PDF in
 * the webview's own document viewer, text as text. For a kind it cannot
 * show — or a picture that would not decode even as the backend's PNG of it
 * (`file.unsupported`) — it says so over the file's tile and offers the save,
 * which is what opening the file comes to then. While the bytes are on their
 * way, a spinner; if they never arrive, the one line that says so — the bar's
 * actions still work on a file that would not open.
 */
export default function Stage({ kind, attachment, file, onSave, onGround }: Props) {
  const { t } = useTranslation()
  const text = useMemo(
    () => (kind === 'text' && file.bytes ? new TextDecoder().decode(file.bytes) : null),
    [kind, file.bytes]
  )
  // Only the ground itself: a press on the picture or the sheet is on the file.
  const ground = (e: MouseEvent<HTMLDivElement>) => {
    if (e.target === e.currentTarget) onGround()
  }

  // Anything that is not the file itself stands in the middle of the stage —
  // on a card of its own on the desktop when it is a block of words (`card`),
  // which the dimmed window behind would otherwise show through.
  const centered = (children: ReactNode, card = false) => (
    <div
      onClick={ground}
      className="flex min-h-0 flex-1 flex-col items-center justify-center px-8 text-center"
    >
      <div
        className={cx(
          'flex flex-col items-center gap-3',
          card && 'md:rounded-lg md:bg-detail md:px-12 md:py-10 md:shadow-float'
        )}
      >
        {children}
      </div>
    </div>
  )

  if (kind === 'none' || file.unsupported)
    return centered(
      <>
        <Thumb attachment={attachment} size="hero" />
        <div>
          <p className="text-base font-medium">{attachment.name}</p>
          <p className={`mt-0.5 ${META}`}>{fileCaption(attachment)}</p>
        </div>
        <p className="text-sm text-text2" data-testid="preview-unavailable">
          {t('No preview for this kind of file')}
        </p>
        <Button variant="ghost" onClick={onSave} className="mt-2">
          {t('Save…')}
        </Button>
      </>,
      true
    )

  if (file.failed)
    return centered(
      <p className="text-base text-bad" role="alert">
        {t('Could not open this file.')}
      </p>
    )

  if (!file.url)
    return centered(
      <span
        aria-label={t('Loading…')}
        className="h-6 w-6 animate-spin rounded-full border-2 border-line border-t-text2"
      />
    )

  if (kind === 'image')
    return (
      // Scrolls, for the picture at its actual size; fitted, there is
      // nothing to scroll.
      <div onClick={ground} className="flex min-h-0 flex-1 overflow-auto md:p-6">
        <Picture key={file.url} url={file.url} name={attachment.name} onError={file.degrade} />
      </div>
    )

  if (kind === 'pdf')
    return (
      <div onClick={ground} className="flex min-h-0 flex-1 flex-col md:px-6">
        <div className={cx('flex min-h-0 flex-1 flex-col overflow-hidden', SHEET)}>
          <iframe
            src={file.url}
            title={attachment.name}
            data-testid="preview-document"
            className="min-h-0 flex-1 border-0 bg-detail"
          />
        </div>
      </div>
    )

  return (
    <div onClick={ground} className="flex min-h-0 flex-1 flex-col md:px-6">
      <pre
        data-testid="preview-text"
        className={cx(
          'min-h-0 flex-1 overflow-auto px-4 py-3 font-mono text-sm break-words whitespace-pre-wrap md:px-6 md:py-5',
          SHEET
        )}
      >
        {text}
      </pre>
    </div>
  )
}
