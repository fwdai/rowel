import { useUi, useVault, selectBatch } from '@/store'
import { useLayout } from '@/hooks/useLayout'

/**
 * Whether a scan under way is shown as the reading screen (`Reading`) — the
 * wide shell's detail pane, the phone's root screen — rather than as a toast
 * (`Status`). One answer for all three, so a scan is always said once and
 * never twice.
 *
 * Not over an open editor: the scan is about to fill that form and must not
 * pull it out from under the user. And not on the phone while an entry is
 * open: it shows one screen at a time, and the entry is the one on top of the
 * root the reading screen is laid on.
 */
export const useReading = (): boolean => {
  const compact = useLayout() === 'compact'
  const busy = useUi(state => state.scanBusy)
  const writing = useVault(state => state.editing || !!state.creating)
  const open = useVault(state => state.currentId !== null && !selectBatch(state))
  return busy && !writing && !(compact && open)
}
