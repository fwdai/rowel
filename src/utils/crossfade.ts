import { flushSync } from 'react-dom'
import { isLinux } from '@/lib/platform'

let latest: ViewTransition | null = null

/**
 * Change what `region` shows as a cross-fade: the region as it was dissolves
 * into the region as `update` — a state change — leaves it (`theme.css` times
 * it). What the two have in common does not change at all on the way, so only
 * what differs is seen to. A second fade ends the first where it stands.
 *
 * `update` simply runs where there are no view transitions, where the user
 * asked for less motion, and on Linux: WebKitGTK has the API and dies of it.
 */
export function crossfade(region: HTMLElement | null, update: () => void): void {
  if (
    !region ||
    isLinux ||
    !('startViewTransition' in document) ||
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  )
    return update()

  // Named only while the fade runs: a named element is a stacking context,
  // and the region's own overlays (a file's preview, a sheet) have to cover
  // the whole window the rest of the time.
  region.style.viewTransitionName = 'swap'
  const transition = document.startViewTransition(() => flushSync(update))
  latest = transition
  // A fade cut short by the next one says so by rejecting; that is expected.
  transition.ready.catch(() => {})
  void transition.finished.finally(() => {
    if (latest === transition) region.style.viewTransitionName = ''
  })
}
