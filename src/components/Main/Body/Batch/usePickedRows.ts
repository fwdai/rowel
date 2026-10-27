import { useMemo } from 'react'
import { useVault } from '@/store'
import { useRows } from '../List/useVisibleEntries'

// The picked rows, read off the view's own rows: a batch only ever acts on
// what the view holds — tombstones in the Archive, live rows everywhere else.
export const usePickedRows = () => {
  const rows = useRows()
  const picked = useVault(state => state.picked)
  return useMemo(() => (picked.size ? rows.filter(row => picked.has(row.id)) : []), [rows, picked])
}
