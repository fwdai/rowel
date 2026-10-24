import { MAX_STANDING_DAYS, MIN_STANDING_DAYS } from '@/api/browser'

/**
 * A number of days as the field holds it, brought back into the range a rule
 * may have. Anything that is not a number — the field emptied mid-edit —
 * falls back to what it was.
 */
export const clampDays = (value: number, fallback: number): number =>
  Number.isFinite(value)
    ? Math.min(MAX_STANDING_DAYS, Math.max(MIN_STANDING_DAYS, Math.round(value)))
    : clampDays(fallback, MIN_STANDING_DAYS)
