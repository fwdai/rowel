import { useEffect, useLayoutEffect, type RefObject } from 'react'
import {
  autoUpdate,
  flip,
  FloatingPortal,
  offset,
  shift,
  size,
  useFloating
} from '@floating-ui/react'
import { cx } from '@/utils/cx'
import { scrollParent } from '@/utils/scrollParent'
import { POPOVER } from '../tokens'
import type { Suggestion } from './match'
import Option from './Option'

interface Props {
  /** The field the list hangs off, and is never wider than. */
  anchor: RefObject<HTMLElement | null>
  /** The listbox's element id, which the field's `aria-controls` names. */
  id: string
  /** Each option's element id, for the field's `aria-activedescendant`. */
  optionId: (index: number) => string
  /** The accessible name: the field's own label. */
  label?: string
  options: Suggestion[]
  /** What is in the field, so each option can show where it answers to it. */
  typed: string
  /** The lit option's index, or -1. One highlight for the arrows and the pointer. */
  lit: number
  onLight: (index: number) => void
  onPick: (value: string) => void
}

// Kept clear of the edges of what it has to stay inside, as a tooltip is.
const EDGE = 8
// Between the field's underline and the list: close enough to read as the
// field's own, not as a menu that happens to be near it.
const GAP = 3
// Wide enough to read an address in. A field narrower than this gets a list
// as wide as itself and no wider: the field's width is the hard limit.
const MIN_WIDTH = 260
// The least the list is cut down to when there is little room on either side
// of the field: two options and the start of a third, which says it scrolls.
const MIN_HEIGHT = 100

// What the list has to stay inside: the pane the field scrolls in, not just
// the window — on the phone that pane ends at the keyboard, which the window
// does not.
const within = (field: Element) => ({
  padding: EDGE,
  boundary: scrollParent(field) ?? ('clippingAncestors' as const)
})

// How far above the field its label starts, when the label is stacked over it
// (the phone's row, a folded one) rather than beside it. A menu that opens
// upward clears the label by this much instead of cutting through its name.
const labelRise = (field: Element): number => {
  const label = (field as HTMLInputElement).labels?.[0]
  if (!label) return 0
  const top = field.getBoundingClientRect().top
  const above = label.getBoundingClientRect()
  return above.bottom <= top + 1 ? top - above.top : 0
}

/**
 * The values a field offers back while it is typed into (see
 * `useSuggestions`, which holds the state): a list dropped out of the field,
 * starting where the field starts and just under its underline, as wide as
 * its longest option and never wider than the field — so it reads as the
 * field's own and cannot cover the sigil before it or the rail after it.
 * Unlike `Dropdown` it never takes the focus and lays no scrim — the caret
 * stays in the field and typing goes on through it — so the options are lit
 * by index, and a press on one is kept from blurring the field it fills.
 *
 * Portaled and placed by floating-ui, as `Tooltip` is: the rows it hangs off
 * sit in cards that clip their overflow, inside a pane that scrolls. It opens
 * above a field that is too near the pane's bottom edge (or the keyboard),
 * and where neither side has room for all of it, it is cut to the room there
 * is and scrolls, the lit option kept in view.
 */
export default function Suggestions({
  anchor,
  id,
  optionId,
  label,
  options,
  typed,
  lit,
  onLight,
  onPick
}: Props) {
  const {
    refs: { setReference, setFloating },
    floatingStyles,
    placement,
    isPositioned
  } = useFloating({
    placement: 'bottom-start',
    // Positioned by top/left rather than a transform, which `animate-drop`'s
    // keyframes would overwrite.
    transform: false,
    whileElementsMounted: autoUpdate,
    middleware: [
      offset(({ placement, elements }) => ({
        mainAxis:
          GAP + (placement.startsWith('top') ? labelRise(elements.reference as Element) : 0)
      })),
      flip(({ elements }) => within(elements.reference as Element)),
      size(({ elements }) => ({
        ...within(elements.reference as Element),
        apply({ rects, availableHeight, elements }) {
          const field = rects.reference.width
          Object.assign(elements.floating.style, {
            minWidth: `${Math.min(MIN_WIDTH, field)}px`,
            maxWidth: `${field}px`,
            maxHeight: `${Math.max(MIN_HEIGHT, availableHeight)}px`
          })
        }
      })),
      shift({ padding: EDGE })
    ]
  })

  useLayoutEffect(() => {
    setReference(anchor.current)
  }, [anchor, setReference])

  // The arrows can walk past the fold of a list that was cut to fit.
  // `nearest`, so an option already showing — any the pointer is on — does
  // not move.
  const litId = lit >= 0 ? optionId(lit) : undefined
  useEffect(() => {
    if (litId) document.getElementById(litId)?.scrollIntoView?.({ block: 'nearest' })
  }, [litId])

  return (
    <FloatingPortal>
      <div
        ref={setFloating}
        style={{ ...floatingStyles, visibility: isPositioned ? undefined : 'hidden' }}
        // A press must not move the focus off the field: its blur is what
        // closes the list, and it would close before the click landed.
        onMouseDown={event => event.preventDefault()}
        className={cx(
          'animate-drop z-50 flex w-max flex-col',
          placement.startsWith('top') ? 'origin-bottom' : 'origin-top',
          POPOVER
        )}
      >
        <div id={id} role="listbox" aria-label={label} className="flex min-h-0 flex-col gap-px overflow-y-auto p-1">
          {options.map((option, index) => (
            <Option
              key={option.value}
              id={optionId(index)}
              option={option}
              typed={typed}
              lit={index === lit}
              onLight={() => onLight(index)}
              onPick={() => onPick(option.value)}
            />
          ))}
        </div>
      </div>
    </FloatingPortal>
  )
}
