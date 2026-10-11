import { useState, type FormEvent } from 'react'
import { createPortal } from 'react-dom'
import { useTranslation } from 'react-i18next'
import { PlusGlyph } from '@/components/Main/icons'
import { useTagCounts } from '@/components/Main/Sidebar/Tags/useTagCounts'
import { cx } from '@/utils/cx'
import Frame from '../Frame'
import MenuRow from '../MenuRow'
import SearchBox from '../SearchBox'
import { CARD, META } from '../tokens'
import Row from './Row'

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
 * with one field and the vault's tags under it, each a row with a check — the
 * way Reminders, Files and Things ask the same question. The field does both
 * things a tag field can: it narrows the rows as it is typed into, and when
 * no row answers to what was typed the first row offers to create it, which
 * Return also does. A name that only differs by case from a tag already there
 * is that tag, not a second one.
 *
 * The rows are the vault's, busiest first (`useTagCounts`), under any the
 * entry carries that the vault does not know yet — the ones just made in this
 * sheet, which stay at the top where they were made rather than jumping into
 * the order once they are counted.
 *
 * Mounted on `document.body`: the compact shell carries `viewportStyle`'s
 * translate, which makes it the containing block of anything `fixed` inside
 * it, and a sheet with a field would take the keyboard offset twice.
 */
export default function TagPicker({ subject, value, onChange, onClose }: Props) {
  const { t } = useTranslation()
  const [query, setQuery] = useState('')
  const counts = useTagCounts()
  const titleId = 'tag-picker-title'

  const known = new Set(counts.map(row => row.tag))
  const rows = [
    ...value.filter(tag => !known.has(tag)).map(tag => ({ tag, count: 0 })),
    ...counts
  ]
  const needle = query.trim()
  const shown = needle ? rows.filter(row => fold(row.tag).includes(fold(needle))) : rows
  // What Return means: the tag already there under that name, or a new one.
  const match = needle ? rows.find(row => fold(row.tag) === fold(needle))?.tag : undefined
  const creatable = needle !== '' && match === undefined

  const toggle = (tag: string, on: boolean) =>
    onChange(on ? [...value, tag] : value.filter(held => held !== tag))

  const create = () => {
    onChange([...value, needle])
    setQuery('')
  }

  const submit = (event: FormEvent) => {
    event.preventDefault()
    if (!needle) return
    if (creatable) return create()
    if (match && !value.includes(match)) toggle(match, true)
    setQuery('')
  }

  return createPortal(
    <Frame onClose={onClose} labelledBy={titleId} testid="tag-picker" fit="content" hideClose>
      {/* The source sheet's rhythm: heading 8px under the grabber, the field
          and the card 12px apart, 20px gutters, the card's last row just above
          the home indicator. */}
      <div className="px-5 pt-2 pb-3">
        <h1 id={titleId} className="truncate text-xl font-semibold tracking-display text-text">
          {t('Tags')}
        </h1>
        {subject && <p className="mt-0.5 truncate text-sm text-text2">{subject}</p>}
      </div>
      {/* A form, so Return in the field is the submit the field has no handler
          for — and nothing else: the editor under the sheet only saves on ⌘⏎. */}
      <form onSubmit={submit} className="px-5 pb-3">
        <SearchBox
          value={query}
          onChange={setQuery}
          placeholder={t('Search or add a tag')}
          testid="tag-picker-input"
          autoFocus
          className="h-11 gap-2.5 rounded-lg pl-3.5 pr-3 [&_input]:text-md"
        />
      </form>
      <div className="px-5 pb-2">
        {rows.length === 0 && !creatable ? (
          <p className={cx('px-1', META)} data-testid="tag-picker-empty">
            {t('Type a name to make your first tag.')}
          </p>
        ) : (
          <div className={CARD}>
            {creatable && (
              <MenuRow
                testid="tag-picker-create"
                label={t('Create “{{tag}}”', { tag: needle })}
                glyph={<PlusGlyph />}
                onClick={create}
              />
            )}
            {shown.map(row => (
              <Row
                key={row.tag}
                tag={row.tag}
                count={row.count}
                on={value.includes(row.tag)}
                onToggle={on => toggle(row.tag, on)}
              />
            ))}
          </div>
        )}
      </div>
    </Frame>,
    document.body
  )
}
