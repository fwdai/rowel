import type { EntryType } from '@/api/types'
import {
  openAddPicker,
  openSettings,
  startEntry,
  setFilterQuery,
  setFilterType
} from '@/store'
import { kindOf } from '@/kinds'
import { chord } from '@/lib/platform'
import { useTranslation } from 'react-i18next'
import EmptyState from '@/components/elements/EmptyState'
import { addEntry } from '@/components/Main/AddSecret/add'
import Mascot from '@/components/elements/Mascot'
import { ActivityGlyph, ArchiveGlyph, SearchGlyph, StarGlyph, TrashGlyph } from '../../icons'
import Mark from './Mark'

// First run — the one hero in the app. Nothing exists yet, so this is the only
// thing on screen worth looking at and it gets the full treatment: the
// lock-screen character itself, pleased to see you, rather than the faceless
// mark every other empty state wears.
export function VaultEmpty() {
  const { t } = useTranslation()

  // No Drive restore here: a vault that syncs holds what its pack holds, so an
  // empty one has nothing up there to pull, and a vault that has never synced
  // reaches the account's vaults through Settings › Sync › Connect.
  return (
    <EmptyState
      testid="empty-vault"
      mark={<Mascot joy={1} size={96} />}
      title={t('Your vault is empty')}
      body={t('Add your first login, card or note. Everything is encrypted before it touches disk.')}
      primary={{
        label: t('Add a secret'),
        onClick: openAddPicker,
        testid: 'create-first-entry-button'
      }}
      // Wrapped, and named: handed straight to onClick it took the click event
      // as its section argument and Settings opened on none at all.
      secondary={{ label: t('Import from another app'), onClick: () => openSettings('import') }}
      hints={[
        { keys: chord('N'), label: t('add') },
        { keys: chord('G'), label: t('generator') },
        { keys: chord('K'), label: t('commands') }
      ]}
    />
  )
}

// The vault has entries and none is open. No body and no buttons — this state
// is one arrow key away from real content — so the space goes to the thing
// worth learning here: every shortcut that answers from where the keyboard is
// now, the list, as a cheat sheet. The list's own keys are `useListKeys`: ⏎
// opens the row and ⌘⏎ copies its secret without opening it. ⌘E is left out —
// it needs an open entry, and this state is the one where there is none.
export function SelectEmpty() {
  const { t } = useTranslation()
  return (
    <EmptyState
      testid="empty-select"
      mark={<Mark />}
      title={t('Select an item')}
      hints={[
        { keys: '↑↓', label: t('browse') },
        { keys: '⏎', label: t('open') },
        { keys: chord('⏎'), label: t('copy') },
        { keys: chord('F'), label: t('search') },
        { keys: chord('N'), label: t('add') },
        { keys: chord('G'), label: t('generator') },
        { keys: chord('K'), label: t('commands') },
        { keys: chord('L'), label: t('lock') }
      ]}
    />
  )
}

// Vault Health with no password to score yet.
export function HealthEmpty() {
  const { t } = useTranslation()
  return (
    <EmptyState
      testid="empty-health"
      mark={<Mark badge={ActivityGlyph} />}
      title={t('Nothing to audit yet')}
      body={t('Your score appears once a login with a password is saved.')}
      primary={{ label: t(kindOf('login').addLabel), onClick: () => startEntry('login') }}
    />
  )
}

// The Favorites view with nothing starred yet — a whole-view state, so it gets
// the pane's full hero and says how to fill itself: the star it takes is the
// one in the entry header, shown as the key to press.
export function FavoritesEmpty() {
  const { t } = useTranslation()
  return (
    <EmptyState
      testid="empty-favorites"
      mark={<Mark badge={StarGlyph} />}
      title={t('No favorites yet')}
      body={t('Star an entry to keep it here.')}
      hints={[{ keys: <StarGlyph size={11} />, label: t('in the entry header') }]}
    />
  )
}

export function ArchiveEmpty() {
  const { t } = useTranslation()
  return (
    <EmptyState
      testid="empty-archive"
      mark={<Mark badge={ArchiveGlyph} />}
      title={t('Nothing archived yet')}
      // One line, like the Favorites body beside it in the rail: the two are
      // a tab apart, and a body that wraps on one and not the other makes the
      // whole surface jump between them.
      body={t('Archived entries wait here, ready to restore.')}
      hints={[{ keys: <TrashGlyph size={11} />, label: t('delete moves an entry here') }]}
    />
  )
}

export function KindEmpty({ type }: { type: EntryType }) {
  const { t } = useTranslation()
  const { Glyph, emptyLabel, addLabel } = kindOf(type)

  return (
    <EmptyState
      compact
      testid="empty-kind"
      mark={<Glyph />}
      title={t(emptyLabel)}
      primary={{ label: t(addLabel), onClick: () => addEntry(type), testid: 'empty-kind-add' }}
    />
  )
}

// A query that matched nothing, naming the query back so it's obvious why the
// list is blank. The kind filter, when one is on, is the other half of the why.
export function SearchEmpty({ query, type }: { query: string; type: EntryType | null }) {
  const { t } = useTranslation()
  const title = type
    ? t(kindOf(type).noMatchesLabel, { query })
    : t('No matches for “{{query}}”', { query })

  return (
    <EmptyState
      compact
      testid="empty-search"
      mark={<SearchGlyph size={16} />}
      title={title}
      primary={{ label: t('Clear search'), onClick: () => setFilterQuery(''), testid: 'empty-search-clear' }}
      // Widening the search is only an option when a kind is narrowing it.
      secondary={
        type
          ? {
              label: t('Search all items'),
              onClick: () => setFilterType(null),
              testid: 'empty-search-widen'
            }
          : undefined
      }
    />
  )
}
