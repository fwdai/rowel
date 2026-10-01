import { useEffect, useRef, type PointerEvent } from 'react'

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
// The rest of the way out once a drag commits: the mount animation's own 160ms,
// and the spring back is `BottomSheet`'s 200ms.
const OUT_MS = 160
const SPRING_MS = 200

interface Origin {
  x: number
  y: number
  at: number
  /** Null until the drag has left the slop: then it is a pop, or a scroll. */
  pop: boolean | null
}

/**
 * The iOS edge swipe, for a pushed screen: a drag in from the left edge takes
 * the screen with the finger, and letting go far or fast enough pops it the
 * same way its back control would — `onBack` is that control's handler, and
 * `disabled` its own flag, so the two can never disagree about whether the
 * screen may be left.
 *
 * The screen is moved by hand rather than through state: a pointermove at
 * 60Hz re-rendering the whole entry screen is a cost the drag would show. Only
 * the pointer handlers and `ref` are React's; the ref callback also sets
 * `touch-action: pan-y`, which is what keeps the webview from claiming a
 * horizontal pan as its own gesture while vertical ones still scroll.
 *
 * There is no exit animation in the shell (the screen under this one simply
 * mounts), so the commit finishes the slide itself before calling back.
 */
export function useSwipeBack(onBack: () => void, disabled = false) {
  const screen = useRef<HTMLDivElement | null>(null)
  const origin = useRef<Origin | null>(null)
  const leaving = useRef<ReturnType<typeof setTimeout> | null>(null)

  // A screen popped while sliding out must not pop twice.
  useEffect(
    () => () => {
      if (leaving.current) clearTimeout(leaving.current)
    },
    []
  )

  const move = (x: number | string, ms: number) => {
    const el = screen.current
    if (!el) return
    el.style.transition = ms ? `translate ${ms}ms ease-out` : 'none'
    el.style.translate = `${x} 0`
  }

  const ref = (el: HTMLDivElement | null) => {
    screen.current = el
    if (el) el.style.touchAction = 'pan-y'
  }

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    // A mouse at the edge of a narrow window is selecting text, not popping.
    if (disabled || leaving.current || e.pointerType === 'mouse') return
    if (e.clientX - e.currentTarget.getBoundingClientRect().left > EDGE_PX) return
    origin.current = { x: e.clientX, y: e.clientY, at: e.timeStamp, pop: null }
  }

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const from = origin.current
    if (!from) return
    const dx = e.clientX - from.x
    const dy = e.clientY - from.y
    if (from.pop === null) {
      if (Math.abs(dx) < SLOP_PX && Math.abs(dy) < SLOP_PX) return
      // Whichever way it went first is what it is. Capturing only now keeps a
      // tap on the back control a click on the back control.
      from.pop = dx > Math.abs(dy)
      if (from.pop) e.currentTarget.setPointerCapture(e.pointerId)
    }
    if (from.pop) move(`${Math.max(0, dx)}px`, 0)
  }

  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    const from = origin.current
    if (!from) return
    origin.current = null
    if (!from.pop) return
    const dx = e.clientX - from.x
    const speed = dx / Math.max(1, e.timeStamp - from.at)
    const width = e.currentTarget.getBoundingClientRect().width
    if (dx > width * COMMIT_SHARE || (dx > FLICK_MIN_PX && speed > FLICK_SPEED)) {
      move('100%', OUT_MS)
      leaving.current = setTimeout(() => {
        leaving.current = null
        onBack()
      }, OUT_MS)
    } else {
      move('0', SPRING_MS)
    }
  }

  const onPointerCancel = () => {
    const from = origin.current
    origin.current = null
    if (from?.pop) move('0', SPRING_MS)
  }

  return { ref, onPointerDown, onPointerMove, onPointerUp, onPointerCancel }
}
