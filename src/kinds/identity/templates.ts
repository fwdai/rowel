import type { TKey } from '@/i18n'
import type { EntryDraft } from '@/kinds/draft'

/**
 * What each kind of ID document is made of.
 *
 * A passport, a driving licence and a residence permit carry overlapping but
 * different sets of the same fields, so the form is data rather than markup:
 * [`FIELDS`] says how one row renders and [`TEMPLATES`] says which rows a
 * document has and which of them it cannot be saved without. `Fields`, `isValid`
 * and the add picker's list of documents all read from here, so a document type
 * is described once.
 */

export const DOC_TYPES = [
  'passport',
  'id_card',
  'driver_license',
  'residence_permit',
  'other'
] as const

export type DocType = (typeof DOC_TYPES)[number]

export const DOC_TYPE_LABELS: Record<DocType, TKey> = {
  passport: 'Passport',
  id_card: 'ID card',
  driver_license: 'Driver license',
  residence_permit: 'Residence permit',
  other: 'Other document'
}

/**
 * The three things a document says, in the order it says them: who is holding
 * it, what the document itself is, and whatever else is printed on it. The rows
 * are banded by these, so a passport reads as three short blocks instead of ten
 * labelled lines.
 */
export type Group = 'holder' | 'document' | 'extra'

/** How one row renders. `text` is the default and carries no flag. */
interface Spec {
  label: TKey
  /** Which band of the panel the row sits in. */
  group: Group
  /** ISO `YYYY-MM-DD`, read and typed in the user's date pattern. */
  date?: boolean
  /** A date the document runs out on: reading it also says how long it has. */
  expiry?: boolean
  /** ISO 3166-1 alpha-3, named in full beside the code while reading. */
  country?: boolean
  /** In the encrypted payload: masked behind a reveal toggle. */
  secret?: boolean
  /** The document's headline value, set 2xl and letter-spaced. */
  big?: boolean
  /** The multi-line body row. */
  note?: boolean
  placeholder?: string
  maxLength?: number
}

// The labels are short because the eyebrow already says what the document is:
// on a passport, "Number" can only be the passport's.
//
// The document number and the personal number are the two fields the vault
// encrypts (see `crypto::sensitive_values`); everything else is printed on the
// document's face and reads plainly.
const FIELDS = {
  name: { label: 'Full name', group: 'holder', placeholder: 'ADA LOVELACE', maxLength: 70 },
  birth_date: { label: 'Born', group: 'holder', date: true },
  sex: { label: 'Sex', group: 'holder', placeholder: 'F', maxLength: 12 },
  nationality: {
    label: 'Nationality',
    group: 'holder',
    country: true,
    placeholder: 'GBR',
    maxLength: 40
  },
  number: {
    label: 'Number',
    group: 'document',
    secret: true,
    big: true,
    placeholder: 'X1234567',
    maxLength: 40
  },
  country: {
    label: 'Country',
    group: 'document',
    country: true,
    placeholder: 'GBR',
    maxLength: 40
  },
  issue_date: { label: 'Issued', group: 'document', date: true },
  expiry_date: { label: 'Expires', group: 'document', date: true, expiry: true },
  authority: { label: 'Authority', group: 'document', placeholder: 'HMPO', maxLength: 60 },
  personal_number: { label: 'Personal no.', group: 'extra', secret: true, maxLength: 40 },
  note: { label: 'Note', group: 'extra', note: true }
} satisfies Record<string, Spec>

export type IdentityKey = keyof typeof FIELDS

export const specOf = (key: IdentityKey): Spec => FIELDS[key]

/** One row of a document's form. */
export interface Row {
  key: IdentityKey
  /** Blocks the save while empty — the document is not itself without it. */
  required?: true
}

// Ordered by band — holder, then document, then whatever else is printed on it
// — and within a band as the document itself reads. Contiguous by construction:
// `Fields` bands the rows by watching the group change down the list.
export const TEMPLATES: Record<DocType, Row[]> = {
  passport: [
    { key: 'name', required: true },
    { key: 'birth_date' },
    { key: 'sex' },
    { key: 'nationality' },
    { key: 'number', required: true },
    { key: 'country', required: true },
    { key: 'issue_date' },
    { key: 'expiry_date', required: true },
    { key: 'authority' },
    { key: 'personal_number' },
    { key: 'note' }
  ],
  id_card: [
    { key: 'name', required: true },
    { key: 'birth_date' },
    { key: 'sex' },
    { key: 'nationality' },
    { key: 'number', required: true },
    { key: 'country', required: true },
    { key: 'issue_date' },
    { key: 'expiry_date' },
    { key: 'authority' },
    { key: 'personal_number' },
    { key: 'note' }
  ],
  driver_license: [
    { key: 'name', required: true },
    { key: 'birth_date' },
    { key: 'number', required: true },
    { key: 'country', required: true },
    { key: 'issue_date' },
    { key: 'expiry_date', required: true },
    { key: 'authority' },
    { key: 'note' }
  ],
  residence_permit: [
    { key: 'name', required: true },
    { key: 'birth_date' },
    { key: 'nationality' },
    { key: 'number', required: true },
    { key: 'country', required: true },
    { key: 'issue_date' },
    { key: 'expiry_date', required: true },
    { key: 'authority' },
    { key: 'note' }
  ],
  other: [
    { key: 'name', required: true },
    { key: 'number', required: true },
    { key: 'country' },
    { key: 'issue_date' },
    { key: 'expiry_date' },
    { key: 'authority' },
    { key: 'note' }
  ]
}

const isDocType = (value: unknown): value is DocType =>
  typeof value === 'string' && (DOC_TYPES as readonly string[]).includes(value)

/**
 * The document type a draft is describing. An unknown or missing one reads as a
 * passport, which is what `defaults` starts from — so a draft from an import or
 * an older vault still renders a form instead of nothing.
 */
export const docTypeOf = (draft: EntryDraft): DocType =>
  isDocType(draft.doc_type) ? draft.doc_type : 'passport'
