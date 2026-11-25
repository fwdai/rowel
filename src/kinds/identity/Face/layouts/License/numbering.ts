import type { IdentityKey } from '../../../templates'

/**
 * The number each field carries on a driving licence. The numbering is the
 * licence's own (ISO/IEC 18013-1) and the same on a card from any country: the
 * front sets each field under its number and the back prints the legend, as
 * the card does.
 */
export const INDEX = {
  name: '1.',
  birth_date: '3.',
  issue_date: '4a.',
  expiry_date: '4b.',
  authority: '4c.',
  number: '5.',
  categories: '9.'
} satisfies Partial<Record<IdentityKey, string>>
