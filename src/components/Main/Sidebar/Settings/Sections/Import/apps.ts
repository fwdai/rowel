import type { TKey } from '@/i18n'
import type { ImportFormat } from '@/api/imports'

// One entry per app an export can come from. The featured ones get a tile of
// their own; the rest sit together in one dropdown row (see `OtherApps`), so
// the pane leads with the apps most people are leaving. A backup of our own is
// not one of them: it has its own row (BackupCard), with its own picker and
// password.
export interface ImportApp {
  key: string
  badge: string
  name: TKey
  hint: TKey
  format: ImportFormat
  featured?: boolean
}

export const APPS: ImportApp[] = [
  {
    key: 'bitwarden',
    badge: 'BW',
    name: 'Bitwarden',
    hint: '.json export',
    format: 'bitwarden',
    featured: true
  },
  {
    key: 'lastpass',
    badge: 'LP',
    name: 'LastPass',
    hint: '.csv export',
    format: 'lastpass',
    featured: true
  },
  {
    key: 'keepass',
    badge: 'KP',
    name: 'KeePass',
    hint: '.csv export',
    format: 'keepass',
    featured: true
  },
  { key: 'chrome', badge: 'CH', name: 'Chrome / Safari', hint: '.csv export', format: 'chrome' },
  { key: 'enpass', badge: 'EP', name: 'Enpass', hint: '.json export', format: 'enpass' },
  {
    key: 'cxf',
    badge: 'CXF',
    name: 'FIDO Credential Exchange (CXF)',
    hint: '.json export',
    format: 'cxf'
  },
  { key: 'csv', badge: 'CSV', name: 'Generic CSV', hint: '.csv export', format: 'csv' }
]

export const FEATURED = APPS.filter(app => app.featured)
export const OTHERS = APPS.filter(app => !app.featured)
