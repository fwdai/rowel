import { t } from '@/i18n'
import type { Entry, EntryMeta } from '@/api/types'
import type { EntryDraft } from '@/kinds/draft'
import { filled } from '@/components/elements/fields/formats'

export const defaults: EntryDraft = {
  type: 'env',
  title: '',
  body: '',
  fileName: '',
  note: ''
}

// The file is the one thing an env entry is; a name without a body has nothing
// to copy or hand back out. The file name is optional — a pasted file has none.
export const isValid = (draft: EntryDraft): boolean =>
  filled(draft.title) && filled(draft.body)

// "Copy .env": the whole file, since one variable is a row's own copy button
// and the file is what a new machine needs.
export const primarySecret = (entry: Entry): string =>
  entry.type === 'env' ? entry.body : ''

// The file name is not a secret, but it is in the encrypted payload — so it can
// only be named once the entry is revealed, and then it belongs next to the
// kind, the way an identity's document type does: `ENV FILE · .env.production`.
// (`fileName` is optional on the wire — a peer's entry may not carry the key.)
export const eyebrow = (entry: EntryDraft) => {
  const raw = entry.type === 'env' ? entry.fileName : ''
  const name = typeof raw === 'string' ? raw.trim() : ''
  return name ? { text: name, testid: 'entry-value-fileName' } : null
}

// `.env.production · 14 vars`, from the metadata stamped at save time (the way
// the card brand is), each part only when known: a pasted file has no name, and
// a row saved before the columns existed has neither until the unlock backfill
// reaches it. With nothing stamped the tags are the only non-secret line left.
export const listSubtitle = (entry: EntryMeta): string => {
  const parts = [
    entry.fileName,
    entry.varCount === undefined ? '' : t('{{count}} vars', { count: entry.varCount })
  ].filter(Boolean)
  return parts.length ? parts.join(' · ') : entry.tags.join(' · ')
}
