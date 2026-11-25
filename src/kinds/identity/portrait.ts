import type { EntryDraft } from '@/kinds/draft'

// What the scanner writes and the core accepts (`models::PORTRAIT_PREFIX`).
const PREFIX = 'data:image/jpeg;base64,'

/**
 * The holder's photograph as something an `<img>` can point at, or undefined.
 *
 * The value arrives from wherever entries do — a sync, an import, another
 * device — so it is drawn only when it is what a portrait is: an inline JPEG.
 * Anything else an `<img>` would fetch or render is left as no photo at all.
 */
export const portraitOf = (entry: EntryDraft): string | undefined => {
  const value = entry.portrait
  return typeof value === 'string' && value.startsWith(PREFIX) ? value : undefined
}
