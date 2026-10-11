import { copyToClipboard } from '@/api/tools'
import { usePrefs } from '@/store'

// Copies a value. Saying so is the caller's: a control that stays on screen
// flashes its own check (`useCopied`), and one that is gone by then raises the
// app-level pill (`flashCopied`).
export const copy = (value: string) => {
  // The backend clears after whatever it is handed, so 0 ("Never") has to reach
  // it as "no timeout at all" rather than "clear immediately".
  copyToClipboard(value, usePrefs.getState().clipboardTimeoutMs || undefined)
}
