import type { EntryDraft } from '@/kinds/draft'
import type { ExtraField } from '@/api/types'

// The list side of the extra fields: reading a draft key as pairs, and deciding
// which of them a save should carry. Kept out of the component file so the
// editor's save path can import it without pulling a component in.

// A draft list is not necessarily extra fields — an entry also carries tags and
// a login its passkeys — so the key is narrowed rather than trusted. `label` is
// the member every extra field has and neither of the others does.
const isExtraField = (value: unknown): value is ExtraField =>
  typeof value === 'object' && value !== null && 'label' in value

export const rowsOf = (value: unknown): ExtraField[] =>
  Array.isArray(value) ? value.filter(isExtraField) : []

/** A row nobody typed anything into is not a field. */
export const isBlank = (field: ExtraField) => !field.label.trim() && !field.value.trim()

/**
 * Whether reading the entry has any custom row to show — for a kind whose read
 * face is an object (a card, a document) rather than a card of rows, which has
 * to know before it draws the panel the rows go on.
 */
export const hasExtra = (entry: EntryDraft) => rowsOf(entry.extra).some(field => !isBlank(field))

/**
 * What a save should send: the rows that say something, and no `extra` key at
 * all when none do — so an entry that never had an extra field stays without
 * one, and a row the user added and left blank never reaches the vault.
 */
export const pruneExtra = (draft: EntryDraft): EntryDraft => {
  if (!('extra' in draft)) return draft
  const kept = rowsOf(draft.extra).filter(field => !isBlank(field))
  if (kept.length > 0) return { ...draft, extra: kept }
  const rest = { ...draft }
  delete rest.extra
  return rest
}
