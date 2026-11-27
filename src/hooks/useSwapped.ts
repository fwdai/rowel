import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react'
import { shallow } from 'zustand/shallow'
import { cascade } from '@/utils/cascade'

// Two swaps closer together than this are someone stepping through a list,
// not opening a view: the second makes no entrance (see `cascade`).
const BRISK = 300

interface Face<T> {
  key: string
  value: T
  /** It follows hard on the one before it. */
  brisk: boolean
}

export interface Swapped<T> {
  /** What to draw: `value`, or the last one that was ready while it is not. */
  shown: T | null
  /** What is drawn is not what was asked for yet, and must not be acted on. */
  settling: boolean
}

/**
 * What a surface shows, changed in one step. While `value` is not `ready` the
 * surface keeps the last one that was, so it never draws a half-loaded state
 * of the next; when the next is ready it replaces the last whole, and makes
 * its entrance (`cascade`, over what `anchor` holds).
 *
 * `key` says which thing `value` is: the same key with other props is the same
 * thing updated, and is adopted at once, with no entrance; another key is a
 * swap.
 */
export function useSwapped<T extends object>(
  value: T,
  key: string,
  ready: boolean,
  anchor: RefObject<HTMLElement | null>
): Swapped<T> {
  const [face, setFace] = useState<Face<T> | null>(ready ? { key, value, brisk: false } : null)
  if (ready && face?.key === key && !shallow(face.value, value)) setFace({ ...face, value })

  // The last ready value: what a swap lands on, read when the swap is due.
  const next = useRef(value)
  useEffect(() => {
    if (ready) next.current = value
  })

  const shownKey = face?.key
  const swappedAt = useRef(0)
  useEffect(() => {
    if (!ready || shownKey === key) return
    const now = performance.now()
    setFace({ key, value: next.current, brisk: now - swappedAt.current < BRISK })
    swappedAt.current = now
  }, [ready, key, shownKey])

  // Before the paint, so the new face is never seen standing still first.
  const brisk = face?.brisk
  useLayoutEffect(() => {
    if (shownKey !== undefined) cascade(anchor.current, brisk)
  }, [shownKey, brisk, anchor])

  const current = ready && shownKey === key
  return {
    shown: current ? value : (face?.value ?? null),
    settling: !!face && !current
  }
}
