import { useEffect, useState } from 'react'
import { useBack } from '@/hooks/useBack'
import { takeSettingsRequest, useUi, type Section } from '@/store'
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
 * One move does come from outside: a section asked for by name
 * (`openSettings('import')`: the empty list's "Import from another app", a
 * transfer Import must take). The store keeps that ask as `settingsRequest`
 * — only ever set while the lock allows the move — and this screen goes there
 * and takes it, so it is one push and not a place Settings is pinned to.
 *
 * Android's Back is whichever nav row is on top: out of a sub-page first, then
 * out of the section, as the wide modal's Escape steps. It is offered here,
 * once, because this is where both pieces of state are held.
 */
export default function Settings() {
  const [section, setSection] = useState<Section | null>(null)
  const locked = useUi(state => state.settingsLocked)
  // A sub-page is one more pane on top of the section's, held the same way.
  const nav = useSubpageState(section)
  // A request pushes its section's pane, whether it opened this screen or
  // found it up. One for the pane already showing moves nothing, as on the
  // wide shell, and leaves its sub-page be.
  const asked = useUi(state => state.settingsRequest)
  useEffect(() => {
    if (!asked) return
    takeSettingsRequest()
    if (asked === section) return
    setSection(asked)
    nav.drop()
  }, [asked, section, nav])
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
