import type { Entry, EntryMeta } from '@/api/types'
import type { EntryDraft } from '@/kinds/draft'
import { emailError, filled } from '@/components/elements/fields/formats'

export const defaults: EntryDraft = {
  type: 'login',
  title: '',
  website: '',
  username: '',
  password: '',
  email: '',
  note: '',
  otp: ''
}

// A passkey IS the credential, so a login carrying one needs no password —
// which is the only shape an imported passkey-only login arrives in. Shared
// with `Fields`, so the row's "Required" and the save agree on when a password
// is one.
export const hasPasskey = (draft: EntryDraft): boolean =>
  Array.isArray(draft.passkeys) && draft.passkeys.length > 0

export const isValid = (draft: EntryDraft): boolean =>
  filled(draft.title) &&
  filled(draft.username) &&
  (filled(draft.password) || hasPasskey(draft)) &&
  // The email is optional, but the row's "Not an email address" complaint is
  // binding: showing it in red and saving anyway is not a check.
  (!filled(draft.email) || !emailError(draft.email))

export const primarySecret = (entry: Entry): string =>
  entry.type === 'login' ? entry.password : ''

export const username = (entry: Entry): string => (entry.type === 'login' ? entry.username : '')

export const url = (entry: Entry): string => (entry.type === 'login' ? entry.website : '')

export const otpSecret = (entry: Entry): string => (entry.type === 'login' ? entry.otp : '')

// The host stands in for a row not yet stamped with its username.
export const listSubtitle = (entry: EntryMeta): string => entry.username || entry.urlHost
