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
import { DropdownItem, DropdownMeta } from '../Dropdown'
import { MENU } from '../tokens'
import { matchAt, type Suggestion } from './match'

interface Props {
  /** The field the list hangs off and is as wide as. */
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
// How far an option's text sits inside the menu: the list's padding plus the
// item's own. The menu starts that far left of the field, so a suggestion is
// printed exactly under what is being typed; it ends where the field ends, so
// it never half-covers the controls in the row's rail.
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

// The value with the stretch that matched in the full ink and the rest a step
// back, so the eye lands on why this one is here. Nothing typed, nothing dimmed.
function Value({ value, typed }: { value: string; typed: string }) {
  const length = typed.trim().length
  const at = length ? matchAt(value, typed) : -1
  if (at < 0) return <span className="min-w-0 flex-1 truncate">{value}</span>

  return (
    <span className="min-w-0 flex-1 truncate text-text2">
      {value.slice(0, at)}
      <span className="text-text">{value.slice(at, at + length)}</span>
      {value.slice(at + length)}
    </span>
  )
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
          elements.floating.style.width = `${rects.reference.width + INSET}px`
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
          'animate-drop z-50 max-w-[calc(100vw-16px)] min-w-[220px]',
          placement.startsWith('top') ? 'origin-bottom' : 'origin-top',
          MENU
        )}
      >
        <div id={id} role="listbox" aria-label={label} className="flex flex-col p-[5px]">
          {options.map((option, index) => (
            <DropdownItem
              key={option.value}
              id={optionId(index)}
              option
              active={index === lit}
              testid={`suggestion-${option.value}`}
              onMouseEnter={() => onLight(index)}
              onClick={() => onPick(option.value)}
            >
              <Value value={option.value} typed={typed} />
              {option.count !== undefined && <DropdownMeta>{option.count}</DropdownMeta>}
            </DropdownItem>
          ))}
        </div>
      </div>
    </FloatingPortal>
  )
}
