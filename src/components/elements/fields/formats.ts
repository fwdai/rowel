import { t } from '@/i18n'

// The pure text rules behind the type-aware fields: what a value becomes when
// the user leaves it, and what the row says when it can't be one.

/**
 * People type "example.com". Give it the scheme the opener and the favicon
 * lookup both need, and leave anything that already has one alone.
 */
export const normalizeUrl = (value: string): string => {
  const url = value.trim()
  if (!url || /^[a-z][a-z\d+.-]*:/i.test(url)) return url
  return `https://${url}`
}

/** The host of a URL as typed, or '' for anything that is not one yet. */
export const hostOf = (url: string): string => {
  try {
    return new URL(url).hostname
  } catch {
    return ''
  }
}

const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/

/** Deliberately shallow: enough to catch a typo, never enough to reject a real address. */
export const emailError = (value: string): string =>
  EMAIL.test(value.trim()) ? '' : t('Not an email address')

/**
 * What every kind's `isValid` counts as a value: present, and not just spaces.
 * A type guard, so the same test narrows a draft key for the format checks.
 */
export const filled = (value?: unknown): value is string =>
  typeof value === 'string' && value.trim() !== ''

/**
 * The one rule behind every "Required" message: a field the kind's `isValid`
 * needs only complains once the user has actually tried to save.
 */
export const requiredError = (
  value: string,
  required?: boolean,
  attempted?: boolean
): string => (attempted && required && !value.trim() ? t('Required') : '')
