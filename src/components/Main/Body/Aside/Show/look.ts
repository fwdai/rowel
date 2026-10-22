import type { EntryType } from '@/api/types'
import type { EntryDraft } from '@/kinds/draft'
import { hostOf } from '@/components/elements/fields/formats'
import { cardBrandOf } from '@/utils/cardBrand'

/**
 * What the tile draws an entry as: the few facts the list's metadata carries
 * for it. An `EntryMeta` is one; a draft is read into one by `lookOf`, so the
 * editor's tile is the read header's, following the fields as they are typed.
 */
export interface Look {
  type: EntryType
  urlHost?: string
  cardBrand?: string
  docType?: string
  docCountry?: string
}

const text = (value: unknown): string => (typeof value === 'string' ? value : '')

// The look of a draft, derived live: the host of the address being typed, the
// brand of the number so far, the document and country a passport is of.
export const lookOf = (draft: EntryDraft): Look => ({
  type: draft.type,
  urlHost: hostOf(text(draft.website) || text(draft.baseUrl)) || undefined,
  cardBrand: draft.type === 'card' ? cardBrandOf(text(draft.number)) : undefined,
  docType: text(draft.doc_type) || undefined,
  docCountry: text(draft.country) || undefined
})

/**
 * The title's face, in both modes: the editor's input takes the same classes
 * over its underline, and the read title wears a transparent one, so the two
 * are the same height and the rows under them do not move by the pixel.
 */
export const TITLE = 'border-b text-2xl font-semibold tracking-display text-text'
