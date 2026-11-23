import { useState } from 'react'
import { useUi, type Section } from '@/store'
import type { SubpageNav } from './sectionNav'
import { subpageSectionOf, type Subpage } from './subpages'

/**
 * The sub-page a shell holds, for the section it is showing (`null` on the
 * phone's root, which shows none).
 *
 * A sub-page belongs to one section, and is only ever returned under it: a
 * shell cannot draw one over another section's place, whatever moved the
 * section — its own nav, a link in a section body, or `openSettings` called
 * from outside the shell (a file the OS opened, the palette), which knows
 * nothing of what the shell holds. One left behind that way is let go here.
 *
 * Under the section lock: while Settings may not be left (`settingsLocked`) a
 * sub-page is neither opened nor left. `drop` is the shell's clear for a pick
 * of the section already showing. Each reads the live lock: its caller may
 * have rendered just before an async operation acquired it.
 */
export function useSubpageState(section: Section | null): SubpageNav & { drop: () => void } {
  const [held, setHeld] = useState<Subpage | null>(null)
  const subpage = held && subpageSectionOf(held) === section ? held : null
  if (held && !subpage) setHeld(null)

  return {
    subpage,
    open: next => {
      // Read the lock at the instant of the action. An async operation can claim
      // it between renders; a callback that captured the prior `false` would let
      // an immediate Escape, scrim click or Back slip through that small gap.
      if (!useUi.getState().settingsLocked) setHeld(next)
    },
    close: () => {
      if (!useUi.getState().settingsLocked) setHeld(null)
    },
    drop: () => {
      if (!useUi.getState().settingsLocked) setHeld(null)
    }
  }
}
