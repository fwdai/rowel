import { useEffect, useRef } from 'react'

// The ways back there are on screen, in the order their surfaces arrived. A
// surface that opens over another mounts after it, so the last is the one on
// top — the one a press of Back belongs to.
const ways: (() => void)[] = []

/**
 * Offer `onBack` as the way back from this surface for as long as it is
 * mounted and `enabled`.
 *
 * This is how a back press that comes from outside the page — Android's
 * button and gesture, through `useBackButton` — reaches the same handler the
 * surface's own control runs: the nav row's back, a dialog's close, the
 * form's guarded Cancel. Nothing is modelled here: each surface says what its
 * control does, and a handler that would refuse the tap (Settings under its
 * lock) refuses the press the same way.
 *
 * A surface registers once, where the state its control changes lives, rather
 * than at each layer it draws: two layers that arrive in one commit run their
 * effects child first, which is not the order they stack in.
 *
 * `onBack` is read through a ref, so a caller may hand in a fresh closure
 * every render without losing its place among the others.
 */
export function useBack(onBack: () => void, enabled = true) {
  const latest = useRef(onBack)
  useEffect(() => {
    latest.current = onBack
  })

  useEffect(() => {
    if (!enabled) return
    const way = () => latest.current()
    ways.push(way)
    return () => {
      ways.splice(ways.indexOf(way), 1)
    }
  }, [enabled])
}

/**
 * Go back from whatever is on top. False when nothing on screen offers a way
 * back, which is the root: the caller decides what leaving it means.
 */
export const goBack = (): boolean => {
  const way = ways[ways.length - 1]
  way?.()
  return way !== undefined
}
