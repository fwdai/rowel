import { useUi, useVault } from '@/store'

/**
 * Whether a scan under way is shown as the reading screen (`Reading`) — the
 * wide shell's detail pane, the phone's whole screen — rather than as a toast
 * (`Status`). Everywhere but over an open editor: the scan is about to fill
 * that form and must not pull it out from under the user. One answer for all
 * three, so a scan is always said once and never twice.
 */
export const useReading = (): boolean => {
  const busy = useUi(state => state.scanBusy)
  const writing = useVault(state => state.editing || !!state.creating)
  return busy && !writing
}
