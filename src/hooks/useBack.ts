import { useEffect, useRef } from 'react'

/**
 * Where a surface sits: a `page` of the shell, or an `overlay` drawn above the
 * pages — a dialog, a sheet, a dropdown, the palette, and a step inside one.
 */
type Layer = 'page' | 'overlay'

interface Way {
  go: () => void
  layer: Layer
}

// The ways back there are on screen, in the order their surfaces arrived. A
// surface that opens over another of its layer mounts after it, so the last of
// a layer is the one on top of it. Across layers the order of arrival says
// nothing: a page can be swapped for another under an open dialog (the read
// screen for the held form, when a reveal fails), and the newcomer is still
// underneath.
const ways: Way[] = []

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
 * `layer` is `overlay` for anything drawn above the pages, which a press
 * reaches before any page whatever order they mounted in.
 *
 * `onBack` is read through a ref, so a caller may hand in a fresh closure
 * every render without losing its place among the others.
 */
export function useBack(onBack: () => void, enabled = true, layer: Layer = 'page') {
  const latest = useRef(onBack)
  useEffect(() => {
    latest.current = onBack
  })

  useEffect(() => {
    if (!enabled) return
    const way: Way = { go: () => latest.current(), layer }
    ways.push(way)
    return () => {
      ways.splice(ways.indexOf(way), 1)
    }
  }, [enabled, layer])
}

/**
 * Go back from whatever is on top: the last overlay, or with none open the
 * last page. False when nothing on screen offers a way back, which is the root: the caller decides what leaving it means.
 */
export const goBack = (): boolean => {
  const last = (layer: Layer) => ways.filter(way => way.layer === layer).pop()
  const way = last('overlay') ?? last('page')
  way?.go()
  return way !== undefined
}
