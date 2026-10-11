import { useRef } from 'react'
import { useTranslation } from 'react-i18next'
import { useScanSupported, closeAddPicker } from '@/store'
import { isMobile } from '@/lib/platform'
import ActionRow from '@/components/elements/ActionRow'
import FileInput from '@/components/elements/FileInput'
import { pickAndScan } from '../Scan/pick'
import { scanCapture } from '../Scan/capture'
import { ScanGlyph } from '../icons'

/**
 * "Scan a card or document…": the picked-file twin of dropping a photo on the
 * window, for the file that is already on disk rather than in hand.
 *
 * On the desktop the row opens the OS file dialog from Rust. On a phone it is
 * the whole of the feature — nothing is ever dropped there — and the photo may
 * not exist yet, so the row opens the system's own sheet (`FileInput`): the
 * camera, the photo library or Files, and what any of them hands back is
 * scanned from its bytes (`Scan/capture`).
 *
 * Deliberately outside the tile grid — the digits and arrows are bound to the
 * tiles, and this is not an nth kind. Where the OS cannot scan it is simply
 * not a row.
 */
export default function ScanAction() {
  const { t } = useTranslation()
  const supported = useScanSupported()
  const input = useRef<HTMLInputElement>(null)

  if (!supported) return null

  // A cancelled dialog leaves the picker as it was; a chosen file hands the
  // window over to the editor the scan is about to fill.
  const pick = async () => {
    if (await pickAndScan()) closeAddPicker()
  }

  const chosen = (file: File) => {
    closeAddPicker()
    void scanCapture(file)
  }

  return (
    <>
      {isMobile && (
        <FileInput ref={input} testid="add-scan-input" accept="image/*" onFile={chosen} />
      )}
      <ActionRow
        testid="add-scan-image"
        glyph={<ScanGlyph size={16} />}
        label={t('Scan a card or document…')}
        caption={
          isMobile
            ? t('Take a photo or choose one from your library. It is read on this device.')
            : t('A photo or screenshot, read on this device.')
        }
        onClick={() => (isMobile ? input.current?.click() : void pick())}
      />
    </>
  )
}
