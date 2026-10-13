import { createContext, useContext, useState } from 'react'
import { setSettingsSection, useUi, type Section } from '@/store'
import type { Subpage } from './subpages'

/**
 * How a section body moves Settings to another section, whichever shell it is
 * drawn in. The wide modal's section lives in the store, so the default is
 * the store's own setter; the phone keeps its pushed section in local state
 * and provides its own (see `Compact/Settings`). A body that only called the
 * store would move the modal nobody is looking at.
 */
const SectionNavContext = createContext<(section: Section) => void>(setSettingsSection)

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
