import { t } from '@/i18n'
import type { Entry, EntryMeta } from '@/api/types'
import type { EntryDraft } from '@/kinds/draft'
import { filled } from '@/components/elements/fields/formats'
import { countryCode } from '@/utils/countries'
import { docTypeOf, DOC_TYPE_LABELS, TEMPLATES } from './templates'

export const defaults: EntryDraft = {
  type: 'identity',
  title: '',
  doc_type: 'passport',
  name: '',
  number: '',
  country: '',
  nationality: '',
  birth_date: '',
  sex: '',
  issue_date: '',
  expiry_date: '',
  authority: '',
  personal_number: '',
  note: ''
}

// What "complete" means depends on the document: a driving licence must have an
// expiry date, an ID card need not. The template is the single answer, so the
// rows' own "Required" marks and the save agree by construction.
export const isValid = (draft: EntryDraft): boolean =>
  filled(draft.title) &&
  TEMPLATES[docTypeOf(draft)]
    .filter(row => row.required)
    .every(row => filled(draft[row.key]))

export const primarySecret = (entry: Entry): string =>
  entry.type === 'identity' ? entry.number : ''

// The document type is in the encrypted payload, so it can only be named once
// the entry is revealed — and then it belongs in the eyebrow, next to the kind,
// rather than on a line of its own above the rows.
export const eyebrow = (entry: EntryDraft) =>
  entry.type === 'identity'
    ? { text: t(DOC_TYPE_LABELS[docTypeOf(entry)]), testid: 'entry-value-doc_type' }
    : null

// The document number is a secret, and a passport's type and country already
// show as its cover in the tile, so there is nothing about the document itself
// to add here; the tags (non-secret metadata) stand in as the secondary line
// when present.
export const listSubtitle = (entry: EntryMeta): string => entry.tags.join(' · ')

/**
 * The issuing country of a passport the list can draw a cover for, read off the
 * row's metadata — or undefined for any other document, and for a passport whose
 * country is not a code we know.
 */
export const passportCountry = (
  entry: Pick<EntryMeta, 'docType' | 'docCountry'>
): string | undefined =>
  entry.docType === 'passport' && entry.docCountry ? countryCode(entry.docCountry) : undefined
