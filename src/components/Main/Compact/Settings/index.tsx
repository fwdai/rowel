import { useState } from 'react'
import { useBack } from '@/hooks/useBack'
import { importWaiting, useApp, useUi, type Section } from '@/store'
import {
  SectionNavProvider,
  SubpageProvider,
  type SectionNav
} from '../../Sidebar/Settings/sectionNav'
import { useSubpageState } from '../../Sidebar/Settings/useSubpageState'
import Root from './Root'
import Pane from './Pane'
import Stack from '../Stack'

/**
 * Settings as a tab root, not an overlay: it has no close button, because the
 * tab bar is how you leave it. A section is one level deeper — a row on the
 * root pushes its pane, and the nav row on the pane comes back.
 *
 * That "root or pane" is local state, not `ui.settingsSection`: the store's
 * section is the *wide* modal's nav selection, which persists so the desktop
 * reopens where it was left. A phone reusing it would open Settings already
 * inside a pane. Local state starts at the root and resets with the screen,
 * which is what a tab root wants — and it is not navigation the store implies,
 * so it earns no place in it.
 *
 * What the store *does* say is when the section may not be left
 * (`settingsLocked`): the wide modal's close and nav refuse under it, and the
 * pane's Back is the same move here, so it refuses the same way. The tab bar
 * needs nothing of its own — leaving Settings from there goes through
 * `closeSettings`, which is already guarded.
 *
 * A section body that sends the user to another section, or to a sub-page of
 * one (the export pane's "save a backup instead"), goes through
 * `SectionNavProvider`, so here it moves this local state rather than the
 * modal's — under the same lock.
 *
 * One move does come from outside: what Import alone can take (another app's
 * data handed over through the system) is sent to `openSettings('import')`,
 * which on the wide shell names the section and here only shows this screen.
 * So while something waits for Import, its pane is the one pushed — once the
 * lock allows — and Import taking it is what ends the wait.
 *
 * Android's Back is whichever nav row is on top: out of a sub-page first, then
 * out of the section, as the wide modal's Escape steps. It is offered here,
 * once, because this is where both pieces of state are held.
 */
export default function Settings() {
  const [section, setSection] = useState<Section | null>(null)
  const locked = useUi(state => state.settingsLocked)
  const waiting = useApp(importWaiting)
  if (waiting && !locked && section !== 'import') setSection('import')
  // A sub-page is one more pane on top of the section's, held the same way.
  const nav = useSubpageState(section)
  const go: SectionNav = (next, page) => {
    if (useUi.getState().settingsLocked) return
    setSection(next)
    if (page) nav.openUnder(next, page)
    else nav.drop()
  }
  const leave = () => {
    if (useUi.getState().settingsLocked) return
    nav.drop()
    setSection(null)
  }
  useBack(nav.subpage ? nav.close : leave, section !== null)

  return (
    <SectionNavProvider value={go}>
      <SubpageProvider value={nav}>
        <Stack
          under={<Root onSelect={setSection} />}
          over={
            section && (
              <Pane section={section} locked={locked} onBack={leave} />
            )
          }
        />
      </SubpageProvider>
    </SectionNavProvider>
  )
}
