import { useEffect, useRef, useState, type RefObject } from 'react'
import { shallow } from 'zustand/shallow'
import { crossfade } from '@/utils/crossfade'
import { scrollParent } from '@/utils/scrollParent'

interface Face<T> {
  key: string
  value: T
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
 * of the next; when the next is ready the one dissolves into the other
 * (`crossfade`, over the scroller `anchor` sits in — a box that keeps its size
 * whatever is put in it).
 *
 * `key` says which thing `value` is: the same key with other props is the same
 * thing updated, and is adopted at once; another key is a swap.
 */
export function useSwapped<T extends object>(
  value: T,
  key: string,
  ready: boolean,
  anchor: RefObject<HTMLElement | null>
): Swapped<T> {
  const [face, setFace] = useState<Face<T> | null>(ready ? { key, value } : null)
  if (ready && face?.key === key && !shallow(face.value, value)) setFace({ key, value })

  // The last ready value: what a swap lands on. The fade starts a frame after
  // it is asked for, and the surface may have been asked for more by then.
  const next = useRef<Face<T> | null>(null)
  useEffect(() => {
    if (ready) next.current = { key, value }
  })

  const shownKey = face?.key
  useEffect(() => {
    if (!ready || shownKey === key) return
    const region = anchor.current && scrollParent(anchor.current)
    crossfade(region, () => setFace(next.current))
  }, [ready, key, shownKey, anchor])

  const current = ready && shownKey === key
  return { shown: current ? value : (face?.value ?? null), settling: !!face && !current }
}
