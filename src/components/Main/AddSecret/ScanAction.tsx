import { useTranslation } from 'react-i18next'
import { useScanSupported, closeAddPicker } from '@/store'
import { isMobile } from '@/lib/platform'
import { pickAndScan } from '../Scan/pick'
import { ScanGlyph } from '../icons'
import ActionRow from './ActionRow'

/**
 * "Scan a card or document…": the picked-file twin of dropping a photo on the
 * window, for the file that is already on disk rather than in hand. On mobile
 * it is the whole of the feature — the dialog opens the Photos picker there,
 * and nothing is ever dropped on a phone — so the copy names a photo.
 *
 * Deliberately outside the tile grid — the digits and arrows are bound to the
 * tiles, and this is not an nth kind. Where the OS cannot scan it is simply
 * not a row.
 */
export default function ScanAction() {
  const { t } = useTranslation()
  const supported = useScanSupported()

  if (!supported) return null

  // A cancelled dialog leaves the picker as it was; a chosen file hands the
  // window over to the editor the scan is about to fill.
  const pick = async () => {
    if (await pickAndScan()) closeAddPicker()
  }

  return (
    <ActionRow
      testid="add-scan-image"
      glyph={<ScanGlyph size={16} />}
      label={isMobile ? t('Scan a photo…') : t('Scan a card or document…')}
      caption={
        isMobile
          ? t('A card or document from your photo library, read on this device.')
          : t('A photo or screenshot, read on this device.')
      }
      onClick={() => void pick()}
    />
  )
}
