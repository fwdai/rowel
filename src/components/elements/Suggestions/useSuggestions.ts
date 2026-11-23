import { useId, useMemo, useState, type KeyboardEvent } from 'react'
import { matching, type Suggestion } from './match'

/**
 * The state of a text field that offers values back as it is typed into: a
 * combobox whose caret never leaves the field. It opens on a keystroke that
 * leaves something in the field — never on focus alone — and nothing is lit
 * until an arrow or the pointer lights it, so Enter and Tab mean what they
 * meant before there were suggestions, unless the user has actually reached
 * for one.
 *
 * With no `source` (a field that offers nothing, or a vault with nothing to
 * offer yet) it is inert: no role, no handlers, an ordinary input.
 */
export function useSuggestions(
  source: Suggestion[] | undefined,
  value: string,
  onPick: (value: string) => void
) {
  const [open, setOpen] = useState(false)
  const [lit, setLit] = useState(-1)
  const base = useId()
  const listId = `${base}-list`
  const optionId = (index: number) => `${base}-${index}`

  const candidates = useMemo(() => (source ? matching(source, value) : []), [source, value])
  const options = open ? candidates : []
  const shown = options.length > 0
  // The list can shrink under a lit row (a sync lands, a letter is typed).
  const active = lit < options.length ? lit : -1

  const show = () => {
    setOpen(true)
    setLit(-1)
  }
  const close = () => setOpen(false)
  const pick = (next: string) => {
    onPick(next)
    close()
  }

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing || event.keyCode === 229) return
    switch (event.key) {
      case 'ArrowDown':
        if (candidates.length === 0) return
        if (shown) setLit((active + 1) % options.length)
        else {
          setOpen(true)
          setLit(0)
        }
        break
      case 'ArrowUp':
        if (!shown) return
        setLit(active <= 0 ? options.length - 1 : active - 1)
        break
      case 'Enter':
        // ⌘⏎ is the editor's save, lit row or not.
        if (active < 0 || event.metaKey || event.ctrlKey) return
        pick(options[active].value)
        break
      case 'Escape':
        if (!shown) return
        // An open list owns Escape: the editor listens on `document` and would
        // otherwise end the whole edit on the same press.
        event.stopPropagation()
        close()
        break
      default:
        return
    }
    event.preventDefault()
  }

  const input = source?.length
    ? ({
        role: 'combobox',
        'aria-autocomplete': 'list',
        'aria-expanded': shown,
        'aria-controls': shown ? listId : undefined,
        'aria-activedescendant': active >= 0 ? optionId(active) : undefined,
        onKeyDown
      } as const)
    : {}

  return { input, shown, options, lit: active, listId, optionId, show, close, pick, light: setLit }
}
