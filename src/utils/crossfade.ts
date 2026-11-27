import { flushSync } from 'react-dom'

// The name the region wears while it fades; `theme.css` times the fade by it.
const NAME = 'swap'

let latest: ViewTransition | null = null

/**
 * Change what `region` shows as a cross-fade: the region as it was dissolves
 * into the region as `update` — a state change — leaves it. What the two have
 * in common (a button in the same place, a card's frame) does not change at
 * all on the way, so only what differs is seen to move.
 *
 * A second swap during a fade ends the first where it stands and fades on from
 * there, so stepping through a list quickly never queues fades up.
 *
 * Where there are no view transitions, or the user asked for less motion,
 * `update` simply runs.
 */
export function crossfade(region: HTMLElement | null, update: () => void): void {
  if (
    !region ||
    !('startViewTransition' in document) ||
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  )
    return update()

  // Named only for as long as the fade runs: a named element is a stacking
  // context, and the region's own overlays (a file's preview, a sheet) have to
  // cover the whole window the rest of the time.
  region.style.viewTransitionName = NAME
  const transition = document.startViewTransition(() => flushSync(update))
  latest = transition
  // A fade cut short by the next one says so by rejecting; that is expected.
  transition.ready.catch(() => {})
  void transition.finished.finally(() => {
    if (latest === transition) region.style.viewTransitionName = ''
  })
}
