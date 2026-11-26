import { daysUntil } from '@/utils/time'

// The three tones a document's validity is said in. Fixed hex like the rest of
// the face: the paper stays light when the app goes dark, so `text-bad` would
// invert underneath it.
const EXPIRED = '#B3261E'
const SOON = '#8A5A00'
const VALID = '#1E7A55'

// Half a year out is where "expires" starts to mean something: most countries
// want six months left on a document before they will let you travel on it.
const SOON_DAYS = 180

/** The ink for a document expiring on `iso`, or null where that is no date. */
export const validityInk = (iso: string): string | null => {
  const days = daysUntil(iso)
  if (days === null) return null
  return days < 0 ? EXPIRED : days <= SOON_DAYS ? SOON : VALID
}

/**
 * How much of a document's term is behind it, 0 to 1 — or null when either end
 * is not a date yet, or the two are the wrong way round.
 */
export const elapsed = (issued: string, expires: string): number | null => {
  const since = daysUntil(issued)
  const left = daysUntil(expires)
  if (since === null || left === null || left <= since) return null
  return Math.min(1, Math.max(0, -since / (left - since)))
}
