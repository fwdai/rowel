import type { ComponentType } from 'react'
import { t } from '@/i18n'
import type { TKey } from '@/i18n'
import type { Section } from '@/store'
import NewWorkspace from './Sections/Workspaces/NewWorkspace/Subpage'
import EditWorkspace from './Sections/Workspaces/EditWorkspace/Subpage'
import DeleteWorkspace from './Sections/Workspaces/DeleteWorkspace/Subpage'
import Nearby from './Sections/Sync/Nearby/Subpage'
import Cloud from './Sections/Sync/Cloud/Subpage'
import Backup from './Sections/Sync/Backup/Subpage'
import { titleOf } from './sections'

export type Subpage =
  | { key: 'new-workspace' }
  | { key: 'edit-workspace'; id: string }
  | { key: 'delete-workspace'; id: string }
  | { key: 'sync-nearby' }
  | { key: 'sync-cloud' }
  | { key: 'sync-backup' }

type Key = Subpage['key']

interface Definition<K extends Key> {
  /** The section it sits under: where it shows, and what the phone's Back is labelled with. */
  crumb: Section
  title: TKey
  description: TKey
  /** Gets its own variant, so a body reads its params (the workspace id) typed. */
  Body: ComponentType<{ subpage: Extract<Subpage, { key: K }> }>
}

// Each sub-page's heading and body, in one place both shells read, the way
// `SECTIONS` is for the sections themselves.
export const SUBPAGES: { [K in Key]: Definition<K> } = {
  'new-workspace': {
    crumb: 'workspaces',
    title: 'New lockbox',
    description: 'Its own encrypted database, unlocked alongside your others.',
    Body: NewWorkspace
  },
  'edit-workspace': {
    crumb: 'workspaces',
    title: 'Edit lockbox',
    description: 'Change how this lockbox is named and told apart.',
    Body: EditWorkspace
  },
  'delete-workspace': {
    crumb: 'workspaces',
    title: 'Delete lockbox',
    description: 'There is no undo, so it takes two proofs first.',
    Body: DeleteWorkspace
  },
  'sync-nearby': {
    crumb: 'sync',
    title: 'Nearby devices',
    description: 'Sync directly between your own devices on the same network.',
    Body: Nearby
  },
  'sync-cloud': {
    crumb: 'sync',
    title: 'Your cloud storage',
    description:
      'Sync through storage you already own. Only encrypted files are ever stored there.',
    Body: Cloud
  },
  'sync-backup': {
    crumb: 'sync',
    title: 'Encrypted backup',
    description: 'A file you keep yourself, sealed with your master password.',
    Body: Backup
  }
}

export const subpageTitleOf = (subpage: Subpage): string => t(SUBPAGES[subpage.key].title)

export const subpageDescriptionOf = (subpage: Subpage): string =>
  t(SUBPAGES[subpage.key].description)

/** The section a sub-page sits under, which is the only one it is shown in. */
export const subpageSectionOf = (subpage: Subpage): Section => SUBPAGES[subpage.key].crumb

/** The parent section's title, as the phone's Back label. */
export const subpageCrumbOf = (subpage: Subpage): string => titleOf(subpageSectionOf(subpage))
