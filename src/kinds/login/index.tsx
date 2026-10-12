import { LoginGlyph } from '@/components/Main/icons'
import type { Kind } from '../types'
import { defaults, isValid, listSubtitle, otpSecret, primarySecret, url, username } from './meta'
import ListRow from './ListRow'
import Fields from './Fields'

const login: Kind = {
  type: 'login',
  label: 'Login',
  pluralLabel: 'Logins',
  description: 'Passwords for apps & sites',
  addLabel: 'Add a login',
  untitledLabel: 'Untitled login',
  emptyLabel: 'No logins yet',
  noMatchesLabel: 'No matches for “{{query}}” in logins',
  Glyph: LoginGlyph,
  tint: 'login',
  defaults,
  isValid,
  primarySecret,
  username,
  url,
  otpSecret,
  primaryActionLabel: 'Copy password',
  listSubtitle,
  ListRow,
  Fields
}

export default login
