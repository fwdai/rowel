import { flushSync } from 'react-dom'

const webkitGtk = navigator.userAgent.includes('Linux')

let latest: ViewTransition | null = null

export function crossfade(region: HTMLElement | null, update: () => void): void {
  if (
    !region ||
    webkitGtk ||
    !('startViewTransition' in document) ||
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  )
    return update()

  region.style.viewTransitionName = 'swap'
  const transition = document.startViewTransition(() => flushSync(update))
  latest = transition
  transition.ready.catch(() => {})
  void transition.finished.finally(() => {
    if (latest === transition) region.style.viewTransitionName = ''
  })
}
