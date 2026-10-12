import { useCallback, useEffect, useRef, useState } from 'react'
import { copy, type CopyOptions } from '@/services/copy'

// How long the confirmation glyph stays up after a copy.
const FEEDBACK_TIMEOUT = 1200

// Copies a value through the clipboard service and raises `copied` for a short
// beat, so every copy affordance (row buttons, the detail header's primary
// action) can flash the same check without owning a timer of its own. The timer
// restarts on a re-copy and is cleared on unmount.
export function useCopied(timeout = FEEDBACK_TIMEOUT) {
  const [copied, setCopied] = useState(false)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)

  useEffect(() => () => clearTimeout(timer.current), [])

  const copyValue = useCallback(
    (value: string, options?: CopyOptions) => {
      copy(value, options)
      clearTimeout(timer.current)
      setCopied(true)
      timer.current = setTimeout(() => setCopied(false), timeout)
    },
    [timeout]
  )

  return { copied, copy: copyValue }
}
