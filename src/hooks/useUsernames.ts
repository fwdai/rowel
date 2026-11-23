import { useMemo } from 'react'
import { useVault } from '@/store'
import { tally } from '@/utils/tally'
import type { Suggestion } from '@/components/elements/Suggestions/match'

/**
 * The usernames the open lockbox's logins already sign in with, most used
 * first. Read off the list metadata, so nothing is revealed to build it — and
 * an address kept only in a login's email row, which is sealed with the rest
 * of the entry, is not among them.
 */
export const useUsernames = (): Suggestion[] => {
  const items = useVault(state => state.items)

  return useMemo(
    () =>
      tally(items.flatMap(item => (item.username ? [item.username] : []))).map(
        ([value, count]) => ({ value, count })
      ),
    [items]
  )
}
