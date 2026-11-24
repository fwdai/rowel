import { useMemo } from 'react'
import { usePrefs } from '@/store'
import { resolveShortcuts } from '@/lib/shortcuts'

/** Every action's key, re-rendering whoever shows one when the user changes it. */
export const useShortcutKeys = () => {
  const overrides = usePrefs(state => state.shortcuts)
  return useMemo(() => resolveShortcuts(overrides), [overrides])
}
