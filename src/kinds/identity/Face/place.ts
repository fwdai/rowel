import { countryName } from '@/utils/countries'
import type { Doc } from './DocCell'

/**
 * The issuing country as a document names it: in full where the field holds a
 * code, and as it was typed where it does not.
 */
export const placeOf = (doc: Doc): string => {
  const country = doc.value('country')
  return countryName(country) ?? country
}
