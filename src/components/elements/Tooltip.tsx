import { useState, type ReactNode } from 'react'
import {
  autoUpdate,
  flip,
  FloatingPortal,
  offset,
  shift,
  useDismiss,
  useFloating,
  useFocus,
  useHover,
  useInteractions
} from '@floating-ui/react'

// Kept clear of the window's edges, so a panel pushed back inside does not sit
// flush against the frame.
const EDGE = 8

interface Props {
  content: ReactNode
  children: ReactNode
  /**
   * The side of the trigger the panel would rather open on: `right` for the
   * left rail, `top` for bottom chrome. It is a preference -- when that side
   * has no room the panel flips to the other one, and it slides along its side
   * to stay inside the window, so a trigger near an edge needs nothing extra.
   */
  side?: 'top' | 'bottom' | 'left' | 'right'
}

export default function Tooltip({ content, children, side = 'bottom' }: Props) {
  const [open, setOpen] = useState(false)
  const {
    refs: { setReference, setFloating },
    floatingStyles,
    context
  } = useFloating({
    open,
    onOpenChange: setOpen,
    placement: side,
    // Positioned by top/left rather than a transform, which `animate-pop`'s
    // keyframes would overwrite.
    transform: false,
    whileElementsMounted: autoUpdate,
    middleware: [offset(8), flip({ padding: EDGE }), shift({ padding: EDGE })]
  })
  // Showing waits 300ms so a pointer crossing a row of tiles doesn't flash
  // every tooltip on the way; hiding is immediate.
  const hover = useHover(context, { delay: { open: 300 }, move: false })
  const focus = useFocus(context)
  const dismiss = useDismiss(context)
  const { getReferenceProps, getFloatingProps } = useInteractions([hover, focus, dismiss])

  return (
    <>
      <div ref={setReference} {...getReferenceProps()}>
        {children}
      </div>
      {open && (
        // Portaled, so no clipping ancestor can cut it off. The trigger already
        // carries the same words as its accessible name, so the panel is kept
        // out of the AT tree rather than announced twice.
        <FloatingPortal>
          <div
            ref={setFloating}
            style={floatingStyles}
            role="tooltip"
            aria-hidden="true"
            className="pointer-events-none z-50 w-max whitespace-nowrap rounded-sm bg-text px-2.5 py-1 text-base text-detail shadow-float animate-pop"
            {...getFloatingProps()}
          >
            {content}
          </div>
        </FloatingPortal>
      )}
    </>
  )
}
