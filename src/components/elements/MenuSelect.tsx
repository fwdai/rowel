import { useRef, useState } from 'react'
import { cx } from '@/utils/cx'
import { ChevronDownGlyph } from '@/components/Main/icons'
import { Dropdown, DropdownCheck, DropdownItem } from './Dropdown'
import { META } from './tokens'

interface Option<T extends string> {
  value: T
  label: string
  // Grey, on the right of the label: a locale code, a pattern.
  meta?: string
}

interface Props<T extends string> {
  // The accessible name; the trigger reads as "<label>: <current option>".
  label: string
  options: Option<T>[]
  value: T
  onChange: (value: T) => void
  // `<prefix>-trigger` on the trigger, `<prefix>-<value>` on each item.
  testidPrefix?: string
}

// The breath the menu keeps from the edge it is measured against.
const MARGIN = 8

// What the menu must stay inside: the nearest ancestor that scrolls (the
// settings modal's section scroller, the phone's pane), else the window. A
// menu that runs past a scroller's starting edge is cut off for good — nothing
// scrolls up to it — so the window's room is not the room there is.
const roomOf = (el: HTMLElement) => {
  for (let node = el.parentElement; node; node = node.parentElement) {
    const overflow = getComputedStyle(node).overflowY
    if (overflow === 'auto' || overflow === 'scroll') return node.getBoundingClientRect()
  }
  return { top: 0, bottom: window.innerHeight }
}

// A single choice from a field-shaped trigger naming the current option, opening
// a menu of all of them. For options too many or too wide to sit side by side.
// On the phone the field spans its row and the menu spans the field; wherever
// it is, the menu opens to the side of the field with more room inside what
// scrolls around it, and is capped to that room, so it never runs off an edge.
export default function MenuSelect<T extends string>({
  label,
  options,
  value,
  onChange,
  testidPrefix
}: Props<T>) {
  const [open, setOpen] = useState(false)
  const [up, setUp] = useState(false)
  const [room, setRoom] = useState<number>()
  const trigger = useRef<HTMLButtonElement>(null)
  const current = options.find(option => option.value === value)

  const toggle = () => {
    if (!open && trigger.current) {
      const { top, bottom } = trigger.current.getBoundingClientRect()
      const bounds = roomOf(trigger.current)
      const above = top - bounds.top
      const below = bounds.bottom - bottom
      setUp(above > below)
      setRoom(Math.max(above, below) - MARGIN)
    }
    setOpen(!open)
  }

  const pick = (next: T) => {
    setOpen(false)
    trigger.current?.focus()
    onChange(next)
  }

  return (
    <div className="relative max-md:w-full">
      <button
        ref={trigger}
        type="button"
        data-testid={testidPrefix && `${testidPrefix}-trigger`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`${label}: ${current?.label ?? value}`}
        onClick={toggle}
        className="flex h-9 w-[236px] max-w-full cursor-pointer items-center gap-2 rounded-sm border border-line2 bg-field px-3 text-base font-medium text-text transition-colors hover:border-accent-line max-md:h-11 max-md:w-full"
      >
        <span className="min-w-0 flex-1 truncate text-left">{current?.label ?? value}</span>
        {current?.meta && <span className={META}>{current.meta}</span>}
        <ChevronDownGlyph className="flex-none text-text2" />
      </button>
      {open && (
        <Dropdown
          onBlur={() => setOpen(false)}
          className={cx(
            'right-0 w-[260px] max-md:left-0 max-md:w-auto',
            up ? 'bottom-full mb-1 origin-bottom' : 'top-full mt-1'
          )}
          listMaxHeight={room}
        >
          {options.map(option => (
            <DropdownItem
              key={option.value}
              checked={option.value === value}
              testid={testidPrefix && `${testidPrefix}-${option.value}`}
              onClick={() => pick(option.value)}
            >
              <span className="min-w-0 flex-1 truncate text-text">{option.label}</span>
              {option.meta && <span className={META}>{option.meta}</span>}
              <DropdownCheck on={option.value === value} />
            </DropdownItem>
          ))}
        </Dropdown>
      )}
    </div>
  )
}
