import { t } from '@/i18n'
import type { Entry, EntryMeta } from '@/api/types'
import type { EntryDraft } from '@/kinds/draft'
import { filled } from '@/components/elements/fields/formats'
import { ENVIRONMENT_LABELS, environmentOf } from './environment'

export const defaults: EntryDraft = {
  type: 'apikey',
  title: '',
  apiKey: '',
  environment: '',
  baseUrl: '',
  scopes: '',
  expiry_date: '',
  note: ''
}

// The token is the entry; where it is sent, for what and until when are notes
// on it. The environment stays optional: not every issuer has two.
export const isValid = (draft: EntryDraft): boolean =>
  filled(draft.title) && filled(draft.apiKey)

export const primarySecret = (entry: Entry): string =>
  entry.type === 'apikey' ? entry.apiKey : ''

// Where the key is sent stands in for a login's website.
export const url = (entry: Entry): string => (entry.type === 'apikey' ? entry.baseUrl : '')

// `API KEY · PRODUCTION`: the environment beside the kind, the way an identity
// names its document — and loud on purpose, since a production key pasted into
// a test script is the mistake this line exists to prevent.
export const eyebrow = (entry: Entry) => {
  const environment = entry.type === 'apikey' ? environmentOf(entry.environment) : null
  return environment
    ? { text: t(ENVIRONMENT_LABELS[environment]), testid: 'entry-eyebrow-environment' }
    : null
}

// Everything about a key — even which API it is for — is in the payload, so,
// as with a note, the tags are the only secondary line the list can draw
// without a reveal.
export const listSubtitle = (entry: EntryMeta): string => entry.tags.join(' · ')
