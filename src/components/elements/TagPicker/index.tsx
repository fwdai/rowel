import { useRef, useState, type KeyboardEvent } from 'react'
import { createPortal } from 'react-dom'
import { useTranslation } from 'react-i18next'
import { useTagCounts } from '@/components/Main/Sidebar/Tags/useTagCounts'
import Frame from '../Frame'
import TagChip from '../TagChip'
import { META } from '../tokens'

interface Props {
  /** What is being filed, under the title: the entry's name. */
  subject?: string
  /** The entry's tags as the sheet holds them. */
  value: string[]
  /** Every press writes the whole list back; what that commits is the caller's. */
  onChange: (next: string[]) => void
  /** Backing out: the scrim, the grabber, or Escape. */
  onClose: () => void
}

const fold = (tag: string) => tag.trim().toLocaleLowerCase()

/**
 * Where an entry's tags are chosen, on a phone: a sheet from the bottom edge
 * holding one token field — the entry's tags as chips inside it, the caret
 * after them — and, floating under it, the rest of the vault's tags as chips
 * to tap into the field, the way Notion, Linear and Mail's recipients field
 * take their tokens. Typing narrows the floating chips; when nothing answers
 * to what was typed, a dashed "Create …" chip stands first, and Return does
 * the same. A name that only differs by case from a tag already there is
 * that tag, not a second one. Backspace in an empty field takes the last tag
 * off, as in any token field. Nothing here is a card: the chips are the
 * content, and a tag looks the same here as it does on the entry.
 *
 * The floating chips are the vault's, busiest first (`useTagCounts`), less
 * those already in the field — a tag is in one place or the other.
 *
 * Mounted on `document.body`: the compact shell carries `viewportStyle`'s
 * translate, which makes it the containing block of anything `fixed` inside
 * it, and a sheet with a field would take the keyboard offset twice.
 */
export default function TagPicker({ subject, value, onChange, onClose }: Props) {
  const { t } = useTranslation()
  const [query, setQuery] = useState('')
  const input = useRef<HTMLInputElement>(null)
  const counts = useTagCounts()
  const titleId = 'tag-picker-title'

  const needle = query.trim()
  // Identity is by folded name throughout: a tag the entry carries as "Work"
  // is the vault's "work", so that one is neither floated nor added twice.
  const held = (tag: string) => value.some(mine => fold(mine) === fold(tag))
  const options = counts.filter(
    row => !held(row.tag) && (!needle || fold(row.tag).includes(fold(needle)))
  )
  // What Return means: the tag already there under that name, or a new one.
  const match = needle
    ? [...value, ...counts.map(row => row.tag)].find(tag => fold(tag) === fold(needle))
    : undefined
  const creatable = needle !== '' && match === undefined
  const bare = counts.length === 0 && value.length === 0 && needle === ''

  // The caret stays in the field through every press on a chip: the next tag
  // is typed, not re-aimed at. A name the field already holds in any case is
  // not added again, whichever path asked.
  const add = (tag: string) => {
    if (!held(tag)) onChange([...value, tag])
    setQuery('')
    input.current?.focus()
  }
  const remove = (tag: string) => {
    onChange(value.filter(held => held !== tag))
    input.current?.focus()
  }

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault()
      if (!needle) return
      if (creatable) return add(needle)
      if (match) return add(match)
      setQuery('')
    } else if (event.key === 'Backspace' && query === '' && value.length > 0) {
      remove(value[value.length - 1])
    } else if (event.key === 'Escape' && query !== '') {
      // Clears what was typed; the dialog's own Escape (which closes) reads
      // `defaultPrevented` and stands down.
      event.preventDefault()
      setQuery('')
    }
  }

  return createPortal(
    <Frame onClose={onClose} labelledBy={titleId} testid="tag-picker" fit="content" hideClose>
      <div className="px-5 pt-2 pb-3">
        <h1 id={titleId} className="truncate text-xl font-semibold tracking-display text-text">
          {t('Tags')}
        </h1>
        {subject && <p className="mt-0.5 truncate text-sm text-text2">{subject}</p>}
      </div>
      {/* The field: the search field's well, holding chips. A press on its
          ground (not on a chip) puts the caret back in it. */}
      <div className="px-5">
        <div
          data-testid="tag-picker-field"
          onClick={event => {
            if (event.target === event.currentTarget) input.current?.focus()
          }}
          className="flex min-h-11 flex-wrap items-center gap-x-2 gap-y-2 rounded-lg border border-line bg-detail px-2.5 py-1.5 transition-[border-color,box-shadow] duration-300 has-[input:focus]:border-accent-line has-[input:focus]:ring-3 has-[input:focus]:ring-accent-soft"
        >
          {value.map(tag => (
            <TagChip
              key={tag}
              tone="field"
              removes
              testid={`tag-applied-${tag}`}
              label={t('Remove tag {{tag}}', { tag })}
              onClick={() => remove(tag)}
            >
              {tag}
            </TagChip>
          ))}
          <input
            ref={input}
            data-testid="tag-picker-input"
            value={query}
            placeholder={t('Add tag')}
            autoFocus
            autoComplete="off"
            autoCorrect="off"
            spellCheck={false}
            enterKeyHint="done"
            onChange={event => setQuery(event.target.value)}
            onKeyDown={onKeyDown}
            className="h-8 min-w-[8ch] flex-1 bg-transparent px-1 text-md text-text caret-accent outline-none placeholder:text-text2"
          />
        </div>
      </div>
      {/* 12px between rows of chips, for the chips' hit slop (see TagChip). */}
      <div className="px-5 pt-4 pb-3">
        {bare ? (
          <p className={META} data-testid="tag-picker-empty">
            {t('Type a name to make your first tag.')}
          </p>
        ) : (
          <div className="flex flex-wrap items-center gap-x-2 gap-y-3">
            {creatable && (
              <TagChip tone="add" testid="tag-picker-create" onClick={() => add(needle)}>
                {t('Create “{{tag}}”', { tag: needle })}
              </TagChip>
            )}
            {options.map(row => (
              <TagChip
                key={row.tag}
                tone="outline"
                testid={`tag-suggest-${row.tag}`}
                meta={row.count}
                onClick={() => add(row.tag)}
              >
                {row.tag}
              </TagChip>
            ))}
          </div>
        )}
      </div>
    </Frame>,
    document.body
  )
}
