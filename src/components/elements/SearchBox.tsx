import type { KeyboardEvent, ReactNode, RefObject } from 'react'
import { cx } from '@/utils/cx'
import { SearchGlyph } from '../Main/icons'

interface Props {
  value: string
  onChange: (value: string) => void
  /** Already translated. */
  placeholder: string
  name?: string
  testid?: string
  /** After the input: the list search puts its ⌘F hint and clear button here. */
  trailing?: ReactNode
  /**
   * The box's measure — height, radius, padding, margin — not a layout flag:
   * the list search is 32px where a mouse points at it and 44px on the compact
   * root, and the env filter sits under a different margin. The surface, its
   * states and their motion are the same field at every size.
   */
  className: string
  /** For a caller that hands the caret back after clearing from outside. */
  inputRef?: RefObject<HTMLInputElement | null>
}

// The search field's surface, with nothing bound to it: the list search wires
// it to the store's query, the env table to its own filter state, and the two
// read as one control. Esc clears the value, then (already clear) blurs — the
// accelerators that act on what is being searched belong to the caller. An Esc
// that cleared something stops here: the editor's document-level Esc is Cancel,
// and clearing a filter must not throw away the edit it sits inside.
export default function SearchBox({
  value,
  onChange,
  placeholder,
  name,
  testid,
  trailing,
  className,
  inputRef
}: Props) {
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== 'Escape') return
    if (value === '') return e.currentTarget.blur()
    e.stopPropagation()
    onChange('')
  }

  return (
    <div
      className={cx(
        'group relative flex items-center border bg-tile text-text3',
        // A flat tinted well with no edge of its own: at rest it reads as a
        // quiet shape in the ground, not a boxed control. Under the pointer the
        // tint deepens a step (ink at a low alpha, so it holds on both themes).
        // Holding the caret it switches on — the tint gives way to the detail
        // surface, a crisp accent stroke draws the edge and a wide, soft halo
        // of the same accent lifts it off the column. Hover stands down while
        // focused so the lit surface never flickers under the pointer. It is
        // the *input's* focus that lights it, not any focus within: the
        // trailing controls (the clear button, a filter set into the field)
        // take focus too, and pressing one is not searching.
        'border-transparent [&:hover:not(:has(input:focus))]:bg-text/10',
        // On the phone the well is the one light thing on the grey ground —
        // the detail surface with a hairline edge, as the prototype draws its
        // field — not a tint darker than the screen, which left the ground the
        // brightest thing in view and made it read as white.
        'max-md:border-line max-md:bg-detail',
        'has-[input:focus]:border-accent-line has-[input:focus]:bg-detail has-[input:focus]:ring-3 has-[input:focus]:ring-accent-soft',
        // The slow tier: a field switching on should be seen doing it.
        'transition-[border-color,background-color,box-shadow] duration-300',
        className
      )}
    >
      {/* The glyph is the field's mood: muted at rest, ink under the pointer,
          accent while the field is listening. */}
      <SearchGlyph className="flex-none transition-colors duration-300 group-hover:text-text2 group-has-[input:focus]:text-accent" />
      <input
        ref={inputRef}
        type="search"
        name={name}
        data-testid={testid}
        placeholder={placeholder}
        value={value}
        autoComplete="off"
        spellCheck={false}
        onChange={e => onChange(e.target.value)}
        onKeyDown={onKeyDown}
        className="min-w-0 flex-1 border-0 bg-transparent text-base text-text caret-accent outline-none placeholder:text-text2 [&::-webkit-search-cancel-button]:hidden"
      />
      {trailing}
    </div>
  )
}
