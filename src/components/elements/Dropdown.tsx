import { useEffect, useRef, type KeyboardEvent, type ReactNode } from 'react'
import { cx } from '@/utils/cx'
import { CheckGlyph } from '@/components/Main/icons'

interface DropdownProps {
  onBlur: () => void
  /**
   * The items' id, given when a search field in the header drives them. The
   * body is then a `listbox` the field controls (a combobox's popup has to
   * be one) and the items are its options, rather than a `menu` of items the
   * arrows focus in turn.
   */
  listbox?: string
  // Placement against the nearest positioned ancestor (e.g. 'right-0 top-8').
  className?: string
  /**
   * Full-bleed above the items, outside their padding: a search field. A menu
   * that has one opens on it rather than on an item.
   */
  header?: ReactNode
  // The items' own scrolling body — a max height, for a list that can run long.
  listClassName?: string
  // The same cap measured at open time, in px: the room the menu has on the
  // side it opens to, inside whatever scrolls around it.
  listMaxHeight?: number
  children: ReactNode
}

export function Dropdown({
  onBlur,
  listbox,
  className,
  header,
  listClassName,
  listMaxHeight,
  children
}: DropdownProps) {
  const ref = useRef<HTMLDivElement>(null)
  const trigger = useRef<HTMLElement | null>(null)

  // Every kind of item: plain, the radio kind a single-select menu uses, and
  // the options of a listbox a search field drives.
  const items = () =>
    Array.from(
      ref.current?.querySelectorAll<HTMLElement>('[role^="menuitem"], [role="option"]') ?? []
    )

  // Roving focus: unlike a radio group the arrows only *move*, they never
  // activate, so the menu holds no selection of its own.
  const move = (step: number) => {
    const all = items()
    if (all.length === 0) return
    const from = all.indexOf(document.activeElement as HTMLElement)
    all[(from + step + all.length) % all.length].focus()
  }

  // The menu takes focus on open so the keyboard lands inside it — on its
  // search field if it has one, else on the item already chosen, else the
  // first — and hands focus back to whatever opened it on Escape.
  useEffect(() => {
    trigger.current = document.activeElement as HTMLElement | null
    const all = items()
    const start =
      ref.current?.querySelector('input') ??
      all.find(item => item.getAttribute('aria-checked') === 'true') ??
      all[0]
    start?.focus()
  }, [])

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    switch (event.key) {
      case 'ArrowDown':
        move(1)
        break
      case 'ArrowUp':
        move(-1)
        break
      case 'Tab':
        // Left unprevented, the Tab moves on from the trigger once the menu is gone.
        trigger.current?.focus()
        onBlur()
        return
      case 'Escape':
        // An open menu owns Escape. Modal and Generator listen on `window`
        // and Sheet on `document` — all ancestors of the React root — so
        // without this the one press would dismiss the overlay underneath too.
        event.stopPropagation()
        onBlur()
        trigger.current?.focus()
        break
      default:
        return
    }
    event.preventDefault()
  }

  return (
    <>
      <div
        ref={ref}
        role={listbox ? undefined : 'menu'}
        onKeyDown={onKeyDown}
        className={cx(
          'animate-drop absolute z-20 flex min-w-[180px] origin-top flex-col overflow-hidden rounded-[12px] border border-menu-line bg-menu text-text shadow-menu backdrop-blur-[24px] backdrop-saturate-[1.6]',
          className
        )}
      >
        {header}
        <div
          id={listbox}
          role={listbox && 'listbox'}
          style={listMaxHeight === undefined ? undefined : { maxHeight: listMaxHeight }}
          className={cx('flex flex-col overflow-y-auto p-[5px]', listClassName)}
        >
          {children}
        </div>
      </div>
      <div
        data-testid="dropdown-scrim"
        className="fixed inset-0 z-10"
        onClick={onBlur}
      />
    </>
  )
}

interface ItemProps {
  // Stable, for a field that names the lit item by `aria-activedescendant`.
  id?: string
  /**
   * An option of the listbox a search field drives (see `Dropdown.listbox`):
   * `aria-selected` says which one the field has lit — `active` — and a
   * `checked` item says it is the chosen one by `aria-checked`.
   */
  option?: boolean
  // A rule across the menu above this item, setting it apart.
  separated?: boolean
  // Destructive entry (delete, disconnect, ...): inked in the `bad` token.
  danger?: boolean
  testid?: string
  onClick?: () => void
  className?: string
  /**
   * Set on every item of a menu that picks one of several (a sort order, a
   * vault): the item becomes a `menuitemradio` and says whether it is the one,
   * so the selection a check glyph shows is also told to assistive technology.
   * Left undefined, the item is a plain action.
   */
  checked?: boolean
  /**
   * Given, the caller owns the highlight and the pointer and focus stop
   * drawing it: a menu whose keyboard stays in a search field moves it by
   * index, and one highlight has to follow both hands (see WorkspacePicker).
   */
  active?: boolean
  onMouseEnter?: () => void
  onFocus?: () => void
  children: ReactNode
}

export function DropdownItem({
  id,
  option,
  separated,
  danger,
  testid,
  onClick,
  className,
  checked,
  active,
  onMouseEnter,
  onFocus,
  children
}: ItemProps) {
  const radio = checked !== undefined
  const highlight =
    active !== undefined
      ? active && (checked ? 'bg-sel' : 'bg-hover')
      : checked
        ? 'bg-sel'
        : 'hover:bg-hover focus-visible:bg-hover'

  return (
    <>
      {separated && <DropdownSeparator />}
      <button
        type="button"
        id={id}
        tabIndex={-1}
        role={option ? 'option' : radio ? 'menuitemradio' : 'menuitem'}
        aria-selected={option ? !!active : undefined}
        aria-checked={checked}
        data-testid={testid}
        onClick={onClick}
        onMouseEnter={onMouseEnter}
        onFocus={onFocus}
        className={cx(
          'flex min-h-8 w-full flex-none cursor-pointer items-center gap-2.5 rounded-[7px] px-[9px] text-left text-base focus-visible:outline-none',
          danger ? 'text-bad' : 'text-text',
          highlight,
          className
        )}
      >
        {children}
      </button>
    </>
  )
}

export function DropdownSeparator() {
  return <div role="separator" className="mx-[9px] my-[5px] h-px flex-none bg-line" />
}

export function DropdownLabel({ children }: { children: ReactNode }) {
  return (
    <div role="presentation" className="px-[9px] pb-1 pt-[5px] text-xs font-medium text-text3">
      {children}
    </div>
  )
}

export function DropdownGlyph({ children }: { children: ReactNode }) {
  return <span className="flex-none opacity-85">{children}</span>
}

export function DropdownCheck({ on }: { on: boolean }) {
  return (
    <span className={cx('grid w-3.5 flex-none place-items-center', !on && 'opacity-0')}>
      <CheckGlyph stroke={2} />
    </span>
  )
}

export function DropdownMeta({
  children,
  testid,
  hint
}: {
  children: ReactNode
  testid?: string
  hint?: boolean
}) {
  return (
    <span
      data-testid={testid}
      aria-hidden={hint || undefined}
      className={cx(
        'flex-none font-mono tabular-nums',
        hint ? 'text-xs opacity-45 any-pointer-coarse:hidden' : 'text-[11.5px] opacity-50'
      )}
    >
      {children}
    </span>
  )
}
