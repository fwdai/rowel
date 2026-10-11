import { createContext, useContext, useEffect, useRef, type PointerEvent } from 'react'

/**
 * What a `Stack` gives the screen over its lower layer: a way to put that
 * layer `share` of the way popped (0 at rest under the screen, 1 clear of it),
 * over `ms` — or at once, under a finger. Null off a stack.
 */
export type Recede = (share: number, ms: number) => void
export const UnderContext = createContext<Recede | null>(null)

// The strip along the left edge a swipe back has to start in, as iOS's own
// interactive pop does: a drag from anywhere else is a scroll or a row.
const EDGE_PX = 24
// Before a drag has gone this far one way or the other it is nothing yet — a
// tap on the back control, or a scroll that has not declared itself.
const SLOP_PX = 8
// How far the screen has to be dragged, as a share of its width, or how fast
// it has to be flicked (px per ms, over at least FLICK_MIN_PX), before letting
// go pops it. The flick numbers are `BottomSheet`'s.
const COMMIT_SHARE = 1 / 3
const FLICK_SPEED = 0.5
const FLICK_MIN_PX = 16
// The rest of the way out once a drag commits: the push's own 320ms over the
// width still to go, so a screen let go near the edge does not crawl, with a
// floor under it so one let go late does not cut. The spring back is
// `BottomSheet`'s 200ms.
const PUSH_MS = 320
const OUT_MIN_MS = 120
const SPRING_MS = 200

interface Origin {
  /** The one finger this drag belongs to; a second one landing is ignored. */
  id: number
  x: number
  y: number
  at: number
  /** The screen's width, measured once: the drag is read as a share of it. */
  width: number
  /** Null until the drag has left the slop: then it is a pop, or a scroll. */
  pop: boolean | null
}

// `theme.css` zeroes every transition under reduced motion, so the slide out
// is over at once and the pop should be too.
const reducedMotion = () =>
  window.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches ?? false

/**
 * The iOS edge swipe, for a pushed screen: a drag in from the left edge takes
 * the screen with the finger, and letting go far or fast enough pops it the
 * same way its back control would — `onBack` is that control's handler, and
 * `disabled` its own flag, so the two can never disagree about whether the
 * screen may be left. Both are read when the pop lands, not when the drag
 * began: a lock that arrives mid-swipe (a restore starting under a settings
 * pane) puts the screen back rather than leaving it off the edge.
 *
 * The screen the finger is going back to is the `Stack` layer under this one.
 * It is moved in step with the drag — out of its receded, dimmed rest and up
 * to full width and brightness as the screen clears — through the `Recede`
 * the stack provides, which writes `--pop`, the one number that layer's
 * geometry is a function of (see `.stack-under`).
 *
 * Both are moved by hand rather than through state: a pointermove at 60Hz
 * re-rendering the whole entry screen is a cost the drag would show. Only the
 * pointer handlers and `ref` are React's; the ref callback also sets
 * `touch-action: pan-y`, which is what keeps the webview from claiming a
 * horizontal pan as its own gesture while vertical ones still scroll.
 *
 * There is no exit animation in the shell (the screen simply unmounts), so
 * the commit finishes the slide itself before calling back.
 */
export function useSwipeBack(onBack: () => void, disabled = false) {
  const screen = useRef<HTMLDivElement | null>(null)
  const recede = useContext(UnderContext)
  const origin = useRef<Origin | null>(null)
  const leaving = useRef<ReturnType<typeof setTimeout> | null>(null)
  // What the back control would do *now*, for a pop that lands after the
  // render that started it.
  const latest = useRef({ onBack, disabled })
  useEffect(() => {
    latest.current = { onBack, disabled }
  })

  // A screen popped while sliding out must not pop twice.
  useEffect(
    () => () => {
      if (leaving.current) clearTimeout(leaving.current)
    },
    []
  )

  // The screen to `x`, and the layer under it to `share` of the way popped,
  // over `ms` — or at once, under the finger.
  const move = (x: string, share: number, ms: number) => {
    const el = screen.current
    if (el) {
      el.style.transition = ms ? `translate ${ms}ms var(--ease-swift)` : 'none'
      el.style.translate = `${x} 0`
    }
    recede?.(share, ms)
  }
  const settle = () => move('0', 0, SPRING_MS)

  // Lets go the way the back control would: through it, or — if it refuses
  // by now — back to where the screen was.
  const pop = () => {
    leaving.current = null
    if (latest.current.disabled) return settle()
    latest.current.onBack()
  }

  const ref = (el: HTMLDivElement | null) => {
    screen.current = el
    if (el) el.style.touchAction = 'pan-y'
  }

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    // A mouse at the edge of a narrow window is selecting text, not popping.
    if (disabled || leaving.current || origin.current || e.pointerType === 'mouse') return
    const { left, width } = e.currentTarget.getBoundingClientRect()
    if (e.clientX - left > EDGE_PX) return
    origin.current = {
      id: e.pointerId,
      x: e.clientX,
      y: e.clientY,
      at: e.timeStamp,
      width: Math.max(1, width),
      pop: null
    }
  }

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const from = origin.current
    if (from?.id !== e.pointerId) return
    const dx = e.clientX - from.x
    const dy = e.clientY - from.y
    if (from.pop === null) {
      if (Math.abs(dx) < SLOP_PX && Math.abs(dy) < SLOP_PX) return
      // Whichever way it went first is what it is. Capturing only now keeps a
      // tap on the back control a click on the back control.
      from.pop = dx > Math.abs(dy)
      if (from.pop) e.currentTarget.setPointerCapture(e.pointerId)
    }
    if (!from.pop) return
    const x = Math.max(0, dx)
    move(`${x}px`, Math.min(1, x / from.width), 0)
  }

  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    const from = origin.current
    if (from?.id !== e.pointerId) return
    origin.current = null
    if (!from.pop) return
    const dx = e.clientX - from.x
    const speed = dx / Math.max(1, e.timeStamp - from.at)
    if (dx > from.width * COMMIT_SHARE || (dx > FLICK_MIN_PX && speed > FLICK_SPEED)) {
      if (reducedMotion()) return pop()
      const left = 1 - Math.min(1, Math.max(0, dx) / from.width)
      const ms = Math.max(OUT_MIN_MS, Math.round(PUSH_MS * left))
      move('100%', 1, ms)
      leaving.current = setTimeout(pop, ms)
    } else {
      settle()
    }
  }

  const onPointerCancel = (e: PointerEvent<HTMLDivElement>) => {
    const from = origin.current
    if (from?.id !== e.pointerId) return
    origin.current = null
    if (from.pop) settle()
  }

  return { ref, onPointerDown, onPointerMove, onPointerUp, onPointerCancel }
}
