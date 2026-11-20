import { useMemo } from 'react'
import { useVault } from '@/store'
import { tally } from '@/utils/tally'

export interface TagCount {
  tag: string
  count: number
}

export const useTagCounts = (): TagCount[] => {
  const items = useVault(state => state.items)

  return useMemo(
    () => tally(items.flatMap(item => item.tags)).map(([tag, count]) => ({ tag, count })),
    [items]
  )
}
