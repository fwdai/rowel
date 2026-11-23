export interface Suggestion {
  value: string
  /** How many entries already hold it; shown at the end of its row when given. */
  count?: number
}

// As many as the menu shows at once: enough for the handful of addresses one
// person signs in with, short enough to never need a scrollbar.
export const LIMIT = 6

const fold = (text: string) => text.trim().toLocaleLowerCase()

/**
 * Where what was typed sits inside a suggestion, case aside, or -1. Shared by
 * the filter and by the row that prints the match in a stronger ink, so the
 * two can never disagree about what matched.
 */
export const matchAt = (value: string, typed: string): number =>
  value.toLocaleLowerCase().indexOf(fold(typed))

/**
 * The suggestions that answer to what was typed, in the order they came in
 * (the caller's ranking) but with the ones that *start* with it first: typing
 * "al" is asking for "alex@…" before "sales@…". The value already in the field
 * is left out — offering it back would change nothing — and an empty field is
 * offered nothing at all: the list answers typing, it does not greet a caret.
 */
export const matching = (source: Suggestion[], typed: string): Suggestion[] => {
  const held = typed.trim()
  if (!held) return []
  const hits = source.filter(option => option.value !== held && matchAt(option.value, typed) >= 0)
  const leading = hits.filter(option => matchAt(option.value, typed) === 0)
  const inside = hits.filter(option => matchAt(option.value, typed) > 0)
  return [...leading, ...inside].slice(0, LIMIT)
}
