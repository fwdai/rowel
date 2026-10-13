import type { KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { SearchGlyph } from '@/components/Main/icons'

interface Props {
  value: string
  onChange: (value: string) => void
  onKeyDown: (event: KeyboardEvent<HTMLInputElement>) => void
  // The listbox the rows are in, and the lit row's element id (none when nothing
  // matches). The caret never leaves the field, so this is how a screen
  // reader hears which vault Enter would open as the arrows move.
  controls: string
  active?: string
}

// The menu's header once there are enough vaults to hunt through: a bare field
// ruled off from the rows, with the key that closes it. A combobox over them:
// the arrows move the lit row, and the field says which one it is.
export default function Search({ value, onChange, onKeyDown, controls, active }: Props) {
  const { t } = useTranslation()
  return (
    <div className="flex h-[42px] flex-none items-center gap-2 border-b border-line px-3.5">
      <SearchGlyph className="flex-none text-text3" stroke={1.4} />
      <input
        type="text"
        role="combobox"
        aria-expanded
        aria-autocomplete="list"
        aria-controls={controls}
        aria-activedescendant={active}
        data-testid="workspace-search"
        aria-label={t('Find a lockbox')}
        placeholder={t('Find a lockbox')}
        value={value}
        onChange={event => onChange(event.target.value)}
        onKeyDown={onKeyDown}
        className="min-w-0 flex-1 bg-transparent text-[13.5px] text-text outline-none placeholder:text-text3"
      />
      <span aria-hidden className="flex-none font-mono text-xs text-text3 any-pointer-coarse:hidden">
        esc
      </span>
    </div>
  )
}
