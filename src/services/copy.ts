import { hideWindow } from '@/api/app'
import { copyToClipboard } from '@/api/tools'
import { isMobile } from '@/lib/platform'
import { usePrefs } from '@/store'

export interface CopyOptions {
  /**
   * The copy is a step in work still going on inside Rowel (the generator), so
   * "Hide Rowel after copying" leaves the window where it is.
   */
  stay?: boolean
}

// Copies a value. Saying so is the caller's: a control that stays on screen
// flashes its own check (`useCopied`), and one that is gone by then raises the
// app-level pill (`flashCopied`).
//
// With "Hide Rowel after copying" on, the window then steps aside for the app
// the value is going to — only once the write has landed, so hiding can never
// cost the copy.
export const copy = async (value: string, { stay = false }: CopyOptions = {}) => {
  const { clipboardTimeoutMs, hideOnCopy } = usePrefs.getState()
  // The backend clears after whatever it is handed, so 0 ("Never") has to reach
  // it as "no timeout at all" rather than "clear immediately".
  await copyToClipboard(value, clipboardTimeoutMs || undefined)
  if (hideOnCopy && !stay && !isMobile) await hideWindow()
}
