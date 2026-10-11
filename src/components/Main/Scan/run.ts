import type { EntryType } from '@/api/types'
import { scanImage, scanImageBytes } from '@/api/tools'
import { errorKind } from '@/api/errors'
import {
  useVault,
  selectCurrent,
  startEntry,
  setPrefill,
  scanStarted,
  scanFinished,
  type ScanError
} from '@/store'
import { cleanFields } from './fields'

// Which kind the detail pane is editing right now, if any — a new entry's
// chosen kind, or the kind of the entry being edited.
const editingKind = (): EntryType | null => {
  const { creating, editing } = useVault.getState()
  if (creating) return creating
  return editing ? (selectCurrent(useVault.getState())?.type ?? null) : null
}

// The backend's failures, classified for the copy.
const reason = (error: unknown): ScanError => {
  switch (errorKind(error)) {
    case 'unrecognized':
      return 'unreadable'
    case 'unsupported':
      return 'unsupported'
    default:
      return 'failed'
  }
}

/**
 * Read one image and land its fields in an editor.
 *
 * The same path for a drop, for a file picked from the dialog (both a path on
 * disk) and for a photo the phone's camera or picker handed the webview (its
 * bytes, since it is on no disk). A scan of the kind already open fills that
 * form in place — a second photo of the same passport should not open a second
 * draft — and anything else starts a new entry of the kind that was
 * recognized. Both go through the vault's `prefill`, which `useDraft` is the
 * only reader of.
 *
 * Never throws: the outcome is the status in the store.
 */
export const runScan = async (image: string | Uint8Array): Promise<void> => {
  scanStarted()
  try {
    const result = await (typeof image === 'string' ? scanImage(image) : scanImageBytes(image))
    const fields = cleanFields(result.fields)
    if (editingKind() === result.kind) setPrefill(fields)
    else startEntry(result.kind, fields)
    scanFinished()
  } catch (error) {
    scanFinished(reason(error))
  }
}
