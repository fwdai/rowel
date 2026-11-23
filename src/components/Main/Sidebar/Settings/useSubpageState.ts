import { useState } from 'react'
import { useUi, type Section } from '@/store'
import type { SubpageNav } from './sectionNav'
import type { Subpage } from './subpages'

interface Held {
  /** The section the pages were opened under, which is the only one they show in. */
  section: Section
  /** Bottom first: each was opened from the one before it. */
  pages: Subpage[]
  turn: SubpageNav['turn']
}

const NONE: Subpage[] = []

export interface SubpageState extends SubpageNav {
  /** A link to another section's sub-page: `page` alone, under `section`. */
  openUnder: (section: Section, page: Subpage) => void
  /** Back to the section's own page, however deep the sub-pages went. */
  drop: () => void
}

/**
 * The sub-pages a shell holds, for the section it is showing (`null` on the
 * phone's root, which shows none).
 *
 * They are a stack under the section they were opened from: a page opened from
 * a sub-page goes over it, and Back leads to the one before, so stepping
 * deeper never moves the section and stepping back never lands somewhere the
 * user did not come from. The same page can therefore be reached under more
 * than one section (restoring a backup, from Lockboxes and from Encrypted
 * backup), and belongs each time to the one it was opened in.
 *
 * A stack is only ever returned under its own section: a shell cannot draw one
 * over another section's place, whatever moved the section — its own nav, a
 * link in a section body, or `openSettings` called from outside the shell (a
 * file the OS opened, the palette), which knows nothing of what the shell
 * holds. One left behind that way is let go here.
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
  const current = held && held.section === section ? held : null
  if (held && !current) setHeld(null)

  const pages = current?.pages ?? NONE
  const unlocked = () => !useUi.getState().settingsLocked

  return {
    pages,
    subpage: pages[pages.length - 1] ?? null,
    turn: current?.turn ?? 'forward',
    open: next => {
      if (section && unlocked()) setHeld({ section, pages: [...pages, next], turn: 'forward' })
    },
    openUnder: (under, page) => {
      if (unlocked()) setHeld({ section: under, pages: [page], turn: 'forward' })
    },
    close: () => {
      if (section && unlocked())
        setHeld(pages.length > 1 ? { section, pages: pages.slice(0, -1), turn: 'back' } : null)
    },
    drop: () => {
      if (unlocked()) setHeld(null)
    }
  }
}
