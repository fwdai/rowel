import { createContext, useContext } from 'react'
import { setSettingsSection, type Section } from '@/store'
import type { Subpage } from './subpages'

/** To a section, or straight to one of its sub-pages. */
export type SectionNav = (section: Section, subpage?: Subpage) => void

/**
 * How a section body moves Settings to another section, whichever shell it is
 * drawn in. Each shell provides its own, since each holds the sub-page (and
 * the phone the section too) in local state; a body that only called the
 * store would move the modal nobody is looking at. Outside a shell the store's
 * setter is all there is, and it lands on the section's own page.
 */
const SectionNavContext = createContext<SectionNav>(setSettingsSection)

export const SectionNavProvider = SectionNavContext.Provider

export const useSectionNav = () => useContext(SectionNavContext)

export interface SubpageNav {
  /** Every sub-page open under the section, bottom first. */
  pages: Subpage[]
  /** The one on top, drawn in place of the section body, if any. */
  subpage: Subpage | null
  /** Whether the page on top was stepped into, or come back to. */
  turn: 'forward' | 'back'
  /** Over whatever is showing: the section's own page, or a sub-page of it. */
  open: (subpage: Subpage) => void
  /** Back to what the sub-page on top was opened from. */
  close: () => void
}

/**
 * A step below a section ("New lockbox" under Lockboxes), opened by the
 * section's own body or by a sub-page of it. Each shell holds them in local
 * state, not the store: the store's section persists so the modal reopens
 * where it was left, and a form half-filled last time is not a place to reopen
 * into. Outside a shell nothing is listening, so the default does nothing.
 */
const SubpageContext = createContext<SubpageNav>({
  pages: [],
  subpage: null,
  turn: 'forward',
  open: () => {},
  close: () => {}
})

export const SubpageProvider = SubpageContext.Provider

export const useSubpage = () => useContext(SubpageContext)
