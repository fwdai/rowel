import { IdentityGlyph } from '@/components/Main/icons'
import type { Kind } from '../types'
import { defaults, eyebrow, isValid, listSubtitle, primarySecret } from './meta'
import { DOC_GLYPHS } from './glyphs'
import { DOC_TYPES, DOC_TYPE_LABELS } from './templates'
import ListRow from './ListRow'
import Fields from './Fields'

const identity: Kind = {
  type: 'identity',
  label: 'Identity',
  pluralLabel: 'Identities',
  description: 'Passports, ID cards & licenses',
  addLabel: 'Add an identity',
  untitledLabel: 'Untitled identity',
  emptyLabel: 'No identities yet',
  noMatchesLabel: 'No matches for “{{query}}” in identities',
  Glyph: IdentityGlyph,
  tint: 'identity',
  defaults,
  isValid,
  primarySecret,
  primaryActionLabel: 'Copy number',
  listSubtitle,
  eyebrow,
  // Which document, asked before the form: the whole form is cut from the
  // answer, so it is settled first rather than switched from a row inside it.
  choices: {
    field: 'doc_type',
    title: 'Which document?',
    options: DOC_TYPES.map(value => ({
      value,
      label: DOC_TYPE_LABELS[value],
      Glyph: DOC_GLYPHS[value]
    }))
  },
  ListRow,
  Fields
}

export default identity
