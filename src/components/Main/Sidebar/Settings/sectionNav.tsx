import { createContext, useContext, useState } from 'react'
import { setSettingsSection, useUi, type Section } from '@/store'
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
  /** The sub-page drawn in place of the section body, if any. */
  subpage: Subpage | null
  open: (subpage: Subpage) => void
  /** Back to the section the sub-page was opened from. */
  close: () => void
}

/**
 * A step below a section ("New lockbox" under Lockboxes), opened by the
 * section's own body. Each shell holds it in local state, not the store: the
 * store's section persists so the modal reopens where it was left, and a form
 * half-filled last time is not a place to reopen into. Outside a shell nothing
 * is listening, so the default does nothing.
 */
const SubpageContext = createContext<SubpageNav>({
  subpage: null,
  open: () => {},
  close: () => {}
})

export const SubpageProvider = SubpageContext.Provider

export const useSubpage = () => useContext(SubpageContext)

/**
 * The sub-page a shell holds, under the section lock: while Settings may not
 * be left (`settingsLocked`) a sub-page is neither opened nor left either.
 * `drop` is the shell's clear for a nav pick or backing out to the phone's root.
 * It checks the live lock too: its caller may have rendered just before an
 * async operation acquired that lock.
 */
export function useSubpageState(): SubpageNav & { drop: () => void } {
  const [subpage, setSubpage] = useState<Subpage | null>(null)
  return {
    subpage,
    open: next => {
      // Read the lock at the instant of the action. An async operation can claim
      // it between renders; a callback that captured the prior `false` would let
      // an immediate Escape, scrim click or Back slip through that small gap.
      if (!useUi.getState().settingsLocked) setSubpage(next)
    },
    close: () => {
      if (!useUi.getState().settingsLocked) setSubpage(null)
    },
    drop: () => {
      if (!useUi.getState().settingsLocked) setSubpage(null)
    }
  }
}
