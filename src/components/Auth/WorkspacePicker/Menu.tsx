import { useEffect, useId, useState, type KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { useApp } from '@/store'
import type { Workspace } from '@/api/types'
import { workspaceLabel } from '@/lib/workspace'
import { Dropdown } from '@/components/elements/Dropdown'
import Row from './Row'
import Search from './Search'

interface Props {
  list: Workspace[]
  active: string | null
  onPick: (id: string) => void
  onClose: () => void
}

// Past this many vaults the menu leads with a field to find one by name.
const SEARCH_FROM = 6

// Every vault on this device, the open one checked. One highlight follows the
// pointer, the arrows and focus alike, and opens on the current vault — so with
// a search field holding the caret the arrows still walk the rows and Enter
// picks the one lit.
export default function Menu({ list, active, onPick, onClose }: Props) {
  const { t } = useTranslation()
  const configured = useApp(state => state.sync.configured)
  const [query, setQuery] = useState('')
  // Element ids for the list and each row, so the search field can point at
  // the lit one; per row by workspace id, so an id holds while the list filters.
  const base = useId()
  const listId = `${base}-list`
  const rowId = (workspace: Workspace) => `${base}-${workspace.id}`

  const searchable = list.length >= SEARCH_FROM

  const needle = query.trim().toLowerCase()
  const rows = list
    .map((workspace, index) => ({ workspace, index, label: workspaceLabel(workspace, t) }))
    .filter(row => !needle || row.label.toLowerCase().includes(needle))

  const [lit, setLit] = useState(() =>
    Math.max(0, rows.findIndex(row => row.workspace.id === active))
  )

  // The lit row stays in view as the arrows walk a list longer than the
  // menu's 300px. `nearest`, so a row already showing — any the pointer is
  // on — does not move.
  const litId = rows[lit] && rowId(rows[lit].workspace)
  useEffect(() => {
    if (litId) document.getElementById(litId)?.scrollIntoView?.({ block: 'nearest' })
  }, [litId])

  // How big each vault is and where it lives, read off what is knowable while
  // it is locked: the count its last open here left, and the open one's live
  // connection or any other's the token file the backend found beside it
  // (`synced`). A vault never opened on this device has no count yet, and
  // says only where it is.
  const about = (workspace: Workspace) => {
    const home =
      (workspace.id === active ? configured : workspace.synced === true)
        ? t('Google Drive')
        : t('This device')
    return workspace.itemCount === undefined
      ? home
      : `${t('{{count}} items', { count: workspace.itemCount })} · ${home}`
  }

  const find = (value: string) => {
    setQuery(value)
    setLit(0)
  }

  // The caret stays in the field, so the arrows and Enter are handled here and
  // kept from the menu's own roving focus; Escape goes on up to close it.
  const onSearchKey = (event: KeyboardEvent<HTMLInputElement>) => {
    switch (event.key) {
      case 'ArrowDown':
        setLit(k => Math.min(rows.length - 1, k + 1))
        break
      case 'ArrowUp':
        setLit(k => Math.max(0, k - 1))
        break
      case 'Enter':
        if (rows[lit]) onPick(rows[lit].workspace.id)
        break
      default:
        return
    }
    event.preventDefault()
    event.stopPropagation()
  }

  return (
    // 320px, short of a phone's screen edges on the narrowest of them.
    <div className="absolute top-full z-20 mt-2 w-[320px] max-w-[calc(100vw-24px)]">
      <Dropdown
        listbox={searchable ? listId : undefined}
        onBlur={onClose}
        className="inset-x-0"
        listClassName="max-h-[300px]"
        header={
          searchable && (
            <Search
              value={query}
              onChange={find}
              onKeyDown={onSearchKey}
              controls={listId}
              active={litId}
            />
          )
        }
      >
        {rows.map((row, k) => (
          <Row
            key={row.workspace.id}
            id={row.workspace.id}
            domId={rowId(row.workspace)}
            option={searchable}
            label={row.label}
            color={row.workspace.color}
            about={about(row.workspace)}
            position={row.index}
            selected={row.workspace.id === active}
            lit={k === lit}
            onLight={() => setLit(k)}
            onPick={() => onPick(row.workspace.id)}
          />
        ))}
        {rows.length === 0 && (
          <div
            data-testid="workspace-search-empty"
            className="px-2.5 py-[18px] text-center text-base text-text2"
          >
            {t('No lockbox matches “{{query}}”', { query })}
          </div>
        )}
      </Dropdown>
    </div>
  )
}
