import { useScanSupported } from '@/store'
import Overlay from './Overlay'
import Status from './Status'
import { useDropScan } from './useDropScan'

/**
 * A photo of a card or an ID document, turned into a filled-in editor.
 *
 * Mounted once by `Main`, so the drop target is the unlocked app as a whole
 * rather than one pane. The same routing off a picked file lives in the "Add a
 * secret" picker (`AddSecret/ScanAction`), and both meet in `run.ts`.
 *
 * Where the OS has no text recognizer there is nothing to offer, so this
 * renders — and listens — for nothing at all.
 */
export default function Scan() {
  const supported = useScanSupported()
  const over = useDropScan(supported)

  if (!supported) return null

  return (
    <>
      {over && <Overlay />}
      <Status />
    </>
  )
}
