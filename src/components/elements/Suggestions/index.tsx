import { useLayoutEffect, type RefObject } from 'react'
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
import { MENU } from '../tokens'
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
// Between the field's underline and the menu.
const GAP = 6
// How far an option sits inside the menu: the list's padding plus the item's
// own. The menu starts that far left of the field, so the options line up
// with what is being typed. It is as wide as its longest option, and never
// wider than the field: it ends at the field's end at most, so it cannot
// half-cover the controls in the row's rail.
const INSET = 14

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
 * `useSuggestions`, which holds the state): the app's menu, hung under the
 * field. Unlike `Dropdown` it never takes the focus and lays no scrim — the
 * caret stays in the field and typing goes on through it — so the options are
 * lit by index, and a press on one is kept from blurring the field it fills.
 *
 * Portaled and placed by floating-ui, as `Tooltip` is: the rows it hangs off
 * sit in cards that clip their overflow, inside a pane that scrolls. It opens
 * above a field that is too near the pane's bottom edge (or the keyboard).
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
          GAP + (placement.startsWith('top') ? labelRise(elements.reference as Element) : 0),
        crossAxis: -INSET
      })),
      // Kept inside the pane the field scrolls in, not just the window: on the
      // phone that pane ends at the keyboard, which the window does not.
      flip(({ elements }) => ({
        padding: EDGE,
        boundary: scrollParent(elements.reference as Element) ?? 'clippingAncestors'
      })),
      size({
        apply({ rects, elements }) {
          elements.floating.style.maxWidth = `${rects.reference.width + INSET}px`
        }
      }),
      shift({ padding: EDGE })
    ]
  })

  useLayoutEffect(() => {
    setReference(anchor.current)
  }, [anchor, setReference])

  return (
    <FloatingPortal>
      <div
        ref={setFloating}
        style={{ ...floatingStyles, visibility: isPositioned ? undefined : 'hidden' }}
        // A press must not move the focus off the field: its blur is what
        // closes the list, and it would close before the click landed.
        onMouseDown={event => event.preventDefault()}
        className={cx(
          'animate-drop z-50 w-max min-w-[280px]',
          placement.startsWith('top') ? 'origin-bottom' : 'origin-top',
          MENU
        )}
      >
        <div id={id} role="listbox" aria-label={label} className="flex flex-col p-[5px]">
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
