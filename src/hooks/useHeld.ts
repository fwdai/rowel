import { useState } from 'react'
import { shallow } from 'zustand/shallow'

/**
 * `value` while it is `ready`, and the last value that was until it is again —
 * null if none ever was. For a surface that should keep what it shows while
 * the next thing loads, and change once, when all of it is in hand.
 */
export function useHeld<T extends object>(value: T, ready: boolean): T | null {
  const [held, setHeld] = useState<T | null>(ready ? value : null)
  if (ready && !shallow(held, value)) setHeld(value)
  return ready ? value : held
}
