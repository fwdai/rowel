import { ApiKeyGlyph } from '@/components/Main/icons'
import type { Kind } from '../types'
import { defaults, eyebrow, isValid, listSubtitle, primarySecret, url } from './meta'
import ListRow from './ListRow'
import Fields from './Fields'

const apikey: Kind = {
  type: 'apikey',
  label: 'API key',
  pluralLabel: 'API keys',
  description: 'Tokens for services & APIs',
  addLabel: 'Add an API key',
  untitledLabel: 'Untitled API key',
  emptyLabel: 'No API keys yet',
  noMatchesLabel: 'No matches for “{{query}}” in API keys',
  Glyph: ApiKeyGlyph,
  tint: 'apikey',
  defaults,
  isValid,
  primarySecret,
  url,
  primaryActionLabel: 'Copy key',
  listSubtitle,
  eyebrow,
  ListRow,
  Fields
}

export default apikey
