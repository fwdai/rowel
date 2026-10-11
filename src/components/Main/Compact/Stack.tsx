import { useLayoutEffect, useRef, type ReactNode } from 'react'
import { UnderContext, type Recede } from './useSwipeBack'

interface Props {
  /** The screen that stays: a tab root, or the pane a sub-page was pushed from. */
  under: ReactNode
  /** The screen over it, or nothing. */
  over?: ReactNode
  /**
   * How the screen over arrives. `push` is iOS's navigation push: in from the
   * right edge, with the layer under it receding a third of the way and
   * dimming. `rise` is a modal form coming up from the bottom: the layer under
   * it only dims, since a form is not a step along a path.
   */
  motion?: 'push' | 'rise'
}

/**
 * Two screens, one over the other — the thing a navigation stack is made of.
 *
 * The lower one stays mounted, and keeps its scroll and its state, while a
 * screen is pushed over it. That is what every native stack does, and what the
 * motion needs: a push slides in *over something*, and an edge swipe drags the
 * screen off *something* — the screen the finger is going back to, receding
 * and dimmed under it, rather than a blank ground. Under a screen the layer is
 * `inert`, so nothing on it can be tapped, focused or read out.
 *
 * The layer's geometry is `.stack-under`'s (`theme.css`), written as one
 * number: `--pop`, how far the screen over it has been popped. The data
 * attributes set its resting value and the push transitions it; a drag writes
 * it inline per frame through the `Recede` handed to the screen over, and
 * whatever the drag leaves inline is cleared the moment the layer is uncovered
 * — before paint, so the layer is where its attributes say and the next push
 * transitions from there.
 */
export default function Stack({ under, over, motion = 'push' }: Props) {
  const ref = useRef<HTMLDivElement | null>(null)
  const covered = Boolean(over)

  useLayoutEffect(() => {
    const el = ref.current
    if (covered || !el) return
    el.style.removeProperty('--pop')
    el.style.removeProperty('transition')
  }, [covered])

  const recede: Recede = (share, ms) => {
    const el = ref.current
    if (!el) return
    el.style.transition = ms ? `--pop ${ms}ms var(--ease-swift)` : 'none'
    el.style.setProperty('--pop', String(share))
  }

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <div
        ref={ref}
        inert={covered}
        data-covered={covered || undefined}
        data-recede={(covered && motion === 'push') || undefined}
        className="stack-under absolute inset-0 flex flex-col"
      >
        {under}
      </div>
      {covered && (
        <UnderContext.Provider value={recede}>
          <div className="absolute inset-0 flex flex-col">{over}</div>
        </UnderContext.Provider>
      )}
    </div>
  )
}
