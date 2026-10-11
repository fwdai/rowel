import { useRef, type ChangeEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { closeAddPicker, closeScanSource } from '@/store'
import ActionRow from '@/components/elements/ActionRow'
import DialogHeader from '@/components/elements/DialogHeader'
import Frame from '@/components/elements/Frame'
import { CARD } from '@/components/elements/tokens'
import { CameraGlyph, ImageGlyph } from '@/components/Main/icons'
import { pickAndScan } from './pick'
import { scanCapture } from './capture'

const TITLE_ID = 'scan-source-title'

/**
 * Where the photo comes from, asked the way a phone asks it — the same sheet
 * the attachment flow asks its source in: an action sheet from the bottom
 * edge, one tappable row per source, the scrim or the grabber to back out. The
 * two rows are the camera and the photo library, under the names the system
 * itself uses for them.
 *
 * The camera is the webview's own photo input with `capture`, which iOS opens
 * on the camera outright; what it takes is a `File` on no disk, scanned from
 * its bytes (`capture.ts`). The library is the same dialog the desktop uses,
 * run from Rust (`pick.ts`): with an all-images filter iOS shows the Photos
 * picker for it, and the path it hands back is granted for the scan.
 *
 * Mounted by `Scan`, after the Add picker in `Main`, so it stacks over the
 * picker that asked (a sheet inside the picker's card would be boxed in by the
 * card's translate). Choosing either source hands the window over to the
 * editor the scan will fill, so both close; backing out of the OS's own
 * surface — the camera dismissed, the library picker cancelled — leaves the
 * question open.
 */
export default function SourceSheet() {
  const { t } = useTranslation()
  const input = useRef<HTMLInputElement>(null)

  const handOver = () => {
    closeScanSource()
    closeAddPicker()
  }

  const captured = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    // So the same photo chosen twice fires a change both times.
    event.target.value = ''
    if (!file) return
    handOver()
    void scanCapture(file)
  }

  const library = async () => {
    if (await pickAndScan()) handOver()
  }

  return (
    <Frame
      onClose={closeScanSource}
      labelledBy={TITLE_ID}
      testid="scan-source-sheet"
      fit="content"
      hideClose
    >
      <input
        ref={input}
        type="file"
        accept="image/*"
        capture="environment"
        onChange={captured}
        data-testid="scan-source-capture"
        className="hidden"
        tabIndex={-1}
        aria-hidden
      />
      <DialogHeader
        id={TITLE_ID}
        title={t('Scan a card or document')}
        onClose={closeScanSource}
        className="px-5"
      />
      <div className="px-5 pt-1 pb-5">
        <div className={CARD}>
          <ActionRow
            testid="scan-source-camera"
            glyph={<CameraGlyph size={16} />}
            label={t('Take Photo')}
            onClick={() => input.current?.click()}
          />
          <ActionRow
            testid="scan-source-library"
            glyph={<ImageGlyph size={16} />}
            label={t('Photo Library')}
            onClick={() => void library()}
          />
        </div>
      </div>
    </Frame>
  )
}
