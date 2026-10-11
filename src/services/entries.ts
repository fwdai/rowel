import Fuse from 'fuse.js'
import type { EntryMeta, EntryType } from '@/api/types'
import { revealEntry } from '@/api/vault'
import { kindOf } from '@/kinds'
import { flashCopied } from '@/store'
import { copy } from './copy'

// Decrypts just this entry (secrets never live in the list metadata) and puts
// the one secret worth a shortcut for its kind on the clipboard, with the usual
// auto-clear timeout. Backs the list search's ⌘⏎, which has no control on
// screen to flash a check on, so the app-level pill is its confirmation.
export const copySecret = (entry: EntryMeta): Promise<void> =>
  revealEntry(entry.id)
    .then(revealed => {
      const value = kindOf(revealed.type).primarySecret(revealed)
      if (!value) return
      copy(value)
      flashCopied()
    })
    .catch(() => {})

interface FilterOptions {
  // null means every kind — the "All Items" view.
  type: EntryType | null
  // null means every tag. Tags compose with the kind filter rather than
  // replacing it, so both are applied in the one pass below.
  tag: string | null
  query: string
}

// Fields a query is matched against. Only list metadata is available here
// (secret fields like passwords/notes live in the encrypted payload); the site
// host stands in for the website.
const SEARCH_KEYS = ['title', 'urlHost', 'username', 'tags']

// The rows the kind and tag filters leave. Unordered: the list's own sort
// (recency or A–Z) is applied downstream, so ordering here would only be thrown
// away.
export const scopeEntries = (entries: EntryMeta[], scope: Omit<FilterOptions, 'query'>) =>
  entries.filter(entry => matchType(entry, scope.type) && matchTag(entry, scope.tag))

export type SearchIndex = Fuse<EntryMeta>

// Fuzzy rank across the searchable metadata (typo-tolerant, relevance-ordered).
// Building the index walks every row, so a caller that searches the same rows
// repeatedly (the list, keystroke by keystroke) builds it once and keeps it.
export const searchIndex = (entries: EntryMeta[]): SearchIndex =>
  new Fuse(entries, { keys: SEARCH_KEYS, threshold: 0.4, ignoreLocation: true })

export const searchEntries = (index: SearchIndex, query: string): EntryMeta[] =>
  index.search(query).map(result => result.item)

export const filterEntries = (entries: EntryMeta[], options: FilterOptions): EntryMeta[] => {
  const scoped = scopeEntries(entries, options)
  const query = options.query.trim()
  if (query === '') return scoped
  return searchEntries(searchIndex(scoped), query)
}

const matchType = (entry: EntryMeta, type: EntryType | null) => !type || entry.type === type

// Exact, case-sensitive: a tag is picked off a list of the tags that exist, not
// typed, so there is nothing to be lenient about.
const matchTag = (entry: EntryMeta, tag: string | null) => !tag || entry.tags.includes(tag)
