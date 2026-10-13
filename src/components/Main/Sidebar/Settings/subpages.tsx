import type { ComponentType } from 'react'
import { t } from '@/i18n'
import type { TKey } from '@/i18n'
import type { Section } from '@/store'
import NewWorkspace from './Sections/Workspaces/NewWorkspace/Subpage'
import EditWorkspace from './Sections/Workspaces/EditWorkspace/Subpage'
import DeleteWorkspace from './Sections/Workspaces/DeleteWorkspace/Subpage'
import { titleOf } from './sections'

export type Subpage =
  | { key: 'new-workspace' }
  | { key: 'edit-workspace'; id: string }
  | { key: 'delete-workspace'; id: string }

type Key = Subpage['key']

interface Definition<K extends Key> {
  /** The section it sits under: what the phone's Back is labelled with. */
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
  }
}

export const subpageTitleOf = (subpage: Subpage): string => t(SUBPAGES[subpage.key].title)

export const subpageDescriptionOf = (subpage: Subpage): string =>
  t(SUBPAGES[subpage.key].description)

/** The parent section's title, as the phone's Back label. */
export const subpageCrumbOf = (subpage: Subpage): string => titleOf(SUBPAGES[subpage.key].crumb)
