import { useState } from 'react'
import { importWaiting, useApp, useUi, type Section } from '@/store'
import type { SubpageNav } from './sectionNav'
import type { Subpage } from './subpages'

interface Held {
  /** The section the page was opened under, which is the only one it shows in. */
  section: Section
  page: Subpage
}

export interface SubpageState extends SubpageNav {
  /** A link to another section's sub-page: `page`, under `section`. */
  openUnder: (section: Section, page: Subpage) => void
  /** The shell's clear, for a pick of the section already showing. */
  drop: () => void
}

/**
 * The sub-page a shell holds, for the section it is showing (`null` on the
 * phone's root, which shows none).
 *
 * A sub-page belongs to the section it was opened from, and Back leads there:
 * stepping deeper never moves the section, and stepping back never lands
 * somewhere the user did not come from. The same page can therefore be reached
 * under more than one section (restoring from a backup: Lockboxes, Sync &
 * backup and Import), and belongs each time to the one it was opened in.
 *
 * It is only ever returned under that section: a shell cannot draw one over
 * another section's place, whatever moved the section — its own nav, a link in
 * a section body, or `openSettings` called from outside the shell (a file the
 * OS opened, the palette), which knows nothing of what the shell holds. One
 * left behind that way is let go here.
 *
 * A page over Import steps aside for a file the OS opens meanwhile, and for
 * another app's transfer handed over then (`importWaiting`), which waits the
 * same way: Import is the section that takes such a file,
 * `openSettings('import')` moves nothing when Import is already showing, and
 * under a page the section does not claim it: it is not there to (the wide
 * shell), or leaves it waiting (the phone, see `useImport`). While Settings is locked the page stays and the file
 * waits, and the page steps aside once the lock is released.
 *
 * Under the section lock: while Settings may not be left (`settingsLocked`) a
 * sub-page is neither opened nor left, and the section cannot move from under
 * one either: the store puts off every move asked for under the lock (see
 * `openSettings`), and the phone's own is guarded the same way. Each action
 * reads the live lock: its caller may have rendered just before an async
 * operation acquired it, and a callback that captured the prior `false` would
 * let an immediate Escape, scrim click or Back slip through that small gap.
 */
export function useSubpageState(section: Section | null): SubpageState {
  const [held, setHeld] = useState<Held | null>(null)
  const locked = useUi(state => state.settingsLocked)
  const waiting = useApp(importWaiting)
  const yields = section === 'import' && waiting && !locked
  const subpage = held && held.section === section && !yields ? held.page : null
  if (held && !subpage) setHeld(null)

  const unlocked = () => !useUi.getState().settingsLocked
  const clear = () => {
    if (unlocked()) setHeld(null)
  }

  return {
    subpage,
    open: page => {
      if (section && unlocked()) setHeld({ section, page })
    },
    openUnder: (under, page) => {
      if (unlocked()) setHeld({ section: under, page })
    },
    close: clear,
    drop: clear
  }
}
