import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react'
import { shallow } from 'zustand/shallow'
import { capture, turn, type Leaving, type Way } from '@/utils/turn'

// Two swaps closer together than this are someone stepping through a list,
// not opening a view: the second is only nudged in (see `turn`).
const BRISK = 300

interface Face<T> {
  key: string
  value: T
  /** Which way the surface turned to get here. */
  way: Way
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
 * of the next; when the next is ready it replaces the last whole, and the one
 * is seen out as the other is seen in (`turn`, over what `anchor` holds).
 *
 * `key` says which thing `value` is: the same key with other props is the same
 * thing updated, and is adopted at once, with no movement; another key is a
 * swap. `rank` is where the thing stands among the ones the surface can show —
 * its row in a list — so a swap knows which way it went; without it, or
 * between two faces of one rank, the swap has no direction.
 */
export function useSwapped<T extends object>(
  value: T,
  key: string,
  ready: boolean,
  anchor: RefObject<HTMLElement | null>,
  rank?: number
): Swapped<T> {
  const [face, setFace] = useState<Face<T> | null>(
    ready ? { key, value, way: 0, brisk: false } : null
  )
  if (ready && face?.key === key && !shallow(face.value, value)) setFace({ ...face, value })

  // The last ready value, and where it stood: what a swap lands on, read when
  // the swap is due.
  const next = useRef({ value, rank })
  const shownKey = face?.key
  // Where the face on screen stands now: a list re-sorted under it moves it.
  const shownRank = useRef<number | undefined>(undefined)
  useEffect(() => {
    if (!ready) return
    next.current = { value, rank }
    if (shownKey === key) shownRank.current = rank
  })

  const swappedAt = useRef(0)
  const left = useRef<Leaving | null>(null)
  useEffect(() => {
    if (!ready || shownKey === key) return
    const now = performance.now()
    const brisk = now - swappedAt.current < BRISK
    const { value, rank } = next.current
    const from = shownRank.current
    const way: Way =
      shownKey === undefined || rank === undefined || from === undefined
        ? 0
        : (Math.sign(rank - from) as Way)
    // The old face is still drawn here; once the state is set it is gone.
    left.current = brisk ? null : capture(anchor.current)
    swappedAt.current = now
    shownRank.current = rank
    setFace({ key, value, way, brisk })
  }, [ready, key, shownKey, anchor])

  // Before the paint, so the new face is never seen standing still first.
  const way = face?.way ?? 0
  const brisk = face?.brisk ?? false
  useLayoutEffect(() => {
    if (shownKey !== undefined) turn(anchor.current, left.current, way, brisk)
    left.current = null
  }, [shownKey, way, brisk, anchor])

  const current = ready && shownKey === key
  return {
    shown: current ? value : (face?.value ?? null),
    settling: !!face && !current
  }
}
