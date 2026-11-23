import type { ComponentType } from 'react'
import { t } from '@/i18n'
import type { TKey } from '@/i18n'
import type { Section } from '@/store'
import NewWorkspace from './Sections/Workspaces/NewWorkspace/Subpage'
import EditWorkspace from './Sections/Workspaces/EditWorkspace/Subpage'
import DeleteWorkspace from './Sections/Workspaces/DeleteWorkspace/Subpage'
import RestoreWorkspace from './Sections/Workspaces/RestoreFromAccount/Subpage'
import RestoreBackup from './Sections/Workspaces/RestoreBackup/Subpage'
import Nearby from './Sections/Sync/Nearby/Subpage'
import Cloud from './Sections/Sync/Cloud/Subpage'
import Backup from './Sections/Sync/Backup/Subpage'
import { titleOf } from './sections'

export type Subpage =
  | { key: 'new-workspace' }
  | { key: 'edit-workspace'; id: string }
  | { key: 'delete-workspace'; id: string }
  | { key: 'restore-workspace' }
  /** `path`: a backup already chosen elsewhere, whose grant still stands. */
  | { key: 'restore-backup'; path?: string }
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
  'restore-workspace': {
    crumb: 'workspaces',
    title: 'Restore from a cloud account',
    description:
      'Sign in and pick one of the lockboxes the account already holds. It arrives as a separate lockbox, unlocked with its own master password and syncing to the same pack.',
    Body: RestoreWorkspace
  },
  'restore-backup': {
    crumb: 'workspaces',
    title: 'Restore a backup',
    description:
      'Pick a .rowel file saved from Settings. It arrives as a separate lockbox, on this device only until you connect it to an account.',
    Body: RestoreBackup
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
