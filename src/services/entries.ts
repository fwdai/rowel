import Fuse from 'fuse.js'
import type { Entry, EntryMeta, EntryType } from '@/api/types'
import { generateOtp } from '@/api/tools'
import { revealEntry } from '@/api/vault'
import { completeEntry, kindOf } from '@/kinds'
import { flashCopied } from '@/store'
import { copy } from './copy'

/** What a copy shortcut can lift off an entry without opening it. */
export type EntryValue = 'secret' | 'username' | 'otp' | 'url'

const VALUES: Record<EntryValue, (entry: Entry) => string | Promise<string>> = {
  secret: entry => kindOf(entry.type).primarySecret(entry),
  username: entry => kindOf(entry.type).username?.(entry) ?? '',
  url: entry => kindOf(entry.type).url?.(entry) ?? '',
  // The seed is never what gets pasted: the backend cuts the current code from it.
  otp: entry => {
    const secret = kindOf(entry.type).otpSecret?.(entry)
    return secret ? generateOtp(secret).then(otp => otp.code) : ''
  }
}

// Decrypts just this entry (secrets never live in the list metadata) and puts
// one of its values on the clipboard, with the usual auto-clear timeout. Backs
// the list's copy chords, which have no control on screen to flash a check on,
// so the app-level pill is their confirmation. An entry without the value, or
// one that will not reveal (a tombstone), copies nothing.
export const copyValue = (entry: EntryMeta, value: EntryValue): Promise<void> =>
  revealEntry(entry.id)
    .then(revealed => VALUES[value](completeEntry(revealed)))
    .then(text => {
      if (!text) return
      copy(text)
      flashCopied()
    })
    .catch(() => {})

/** The one secret worth a shortcut for the entry's kind — the list search's ⌘⏎. */
export const copySecret = (entry: EntryMeta): Promise<void> => copyValue(entry, 'secret')

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
