import { useState, type KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import TagChip from './TagChip'
import { META_TYPE } from './tokens'

interface Props {
  value: string[]
  onChange: (tags: string[]) => void
  /** Translated; an empty box otherwise says nothing about what goes in it. */
  placeholder?: string
}

// The desktop editor's tags: type + Enter to add, press a chip to remove it.
// The chips are the read view's chips, so committing a tag doesn't restyle it.
// The phone does not type tags into a line; it picks them (`TagPicker`).
export default function TagsInput({ value, onChange, placeholder }: Props) {
  const { t } = useTranslation()
  const [input, setInput] = useState('')

  const addTag = () => {
    const tag = input.trim()
    if (tag && !value.includes(tag)) onChange([...value, tag])
    setInput('')
  }

  const removeTag = (tag: string) => onChange(value.filter(t => t !== tag))

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault()
      addTag()
    } else if (event.key === 'Backspace' && input === '' && value.length) {
      removeTag(value[value.length - 1])
    }
  }

  return (
    <div className="flex flex-wrap items-center gap-2">
      {value.map(tag => (
        <TagChip
          key={tag}
          tone="field"
          removes
          label={t('Remove tag {{tag}}', { tag })}
          onClick={() => removeTag(tag)}
        >
          {tag}
        </TagChip>
      ))}
      <input
        // The one input in the entry editor with no `name` — specs address it here.
        data-testid="tags-input"
        placeholder={placeholder}
        className={`h-6 min-w-[110px] flex-1 border-b border-line2 bg-transparent ${META_TYPE} text-text outline-none transition-colors placeholder:text-text2 focus:border-accent-line`}
        value={input}
        onChange={e => setInput(e.target.value)}
        onKeyDown={handleKeyDown}
        onBlur={addTag}
      />
    </div>
  )
}
