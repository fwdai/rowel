import type { ComponentType } from 'react'
import type { Section as Key } from '@/store'
import Sync from './Sections/Sync'
import Security from './Sections/Security'
import Workspaces from './Sections/Workspaces'
import Audit from './Sections/Audit'
import Import from './Sections/Import'
import Language from './Sections/Language'
import Browser from './Sections/Browser'

// One place that maps a section key to its pane, so the modal and the compact
// sheet cannot drift apart over which section shows what.
const PANES: Record<Key, ComponentType> = {
  sync: Sync,
  security: Security,
  workspaces: Workspaces,
  audit: Audit,
  import: Import,
  language: Language,
  browser: Browser
}

export default function Section({ section }: { section: Key }) {
  const Pane = PANES[section]
  return <Pane />
}
