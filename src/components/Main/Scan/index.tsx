import { useTranslation } from 'react-i18next'
import { useScanSupported, useUi, closeScanSource } from '@/store'
import SourceSheet from '@/components/elements/SourceSheet'
import Overlay from './Overlay'
import Status from './Status'
import { pickAndScan } from './pick'
import { scanCapture } from './capture'
import { useDropScan } from './useDropScan'

/**
 * A photo of a card or an ID document, turned into a filled-in editor.
 *
 * Mounted once by `Main`, so the drop target is the unlocked app as a whole
 * rather than one pane. The same routing off a picked file lives in the "Add a
 * secret" picker (`AddSecret/ScanAction`), and both meet in `run.ts`. On a
 * phone that picker's row first asks where the photo comes from — the source
 * sheet here, opened in the picker's place. A chosen source hands the window
 * over to the editor the scan will fill, so the sheet closes; a system picker
 * backed out of leaves the question open. A lock drops it with the rest of
 * the session's UI (`resetUi`).
 *
 * Where the OS has no text recognizer there is nothing to offer, so this
 * renders — and listens — for nothing at all.
 */
export default function Scan() {
  const { t } = useTranslation()
  const supported = useScanSupported()
  const over = useDropScan(supported)
  const source = useUi(state => state.scanSource)

  if (!supported) return null

  const pick = async (browse: boolean) => {
    if (await pickAndScan(browse)) closeScanSource()
  }

  const captured = (file: File) => {
    closeScanSource()
    void scanCapture(file)
  }

  return (
    <>
      {over && <Overlay />}
      {source && (
        <SourceSheet
          testid="scan-source"
          title={t('Scan a card or document')}
          description={t(
            'Take a photo or choose one from your library. It is read on this device.'
          )}
          onClose={closeScanSource}
          onCapture={captured}
          onLibrary={() => void pick(false)}
          onBrowse={() => void pick(true)}
        />
      )}
      <Status />
    </>
  )
}
