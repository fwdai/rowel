import { useState } from 'react'
import { useUi, type Section } from '@/store'
import {
  SectionNavProvider,
  SubpageProvider,
  useSubpageState
} from '../../Sidebar/Settings/sectionNav'
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
 * A section body that sends the user to another section (the export pane's
 * "save a backup instead") goes through `SectionNavProvider`, so here it moves
 * this local state rather than the modal's — under the same lock.
 */
export default function Settings() {
  const [section, setSection] = useState<Section | null>(null)
  const locked = useUi(state => state.settingsLocked)
  // A sub-page is one more pane on top of the section's, held the same way.
  const nav = useSubpageState()
  const go = (next: Section) => {
    if (useUi.getState().settingsLocked) return
    nav.drop()
    setSection(next)
  }

  return (
    <SectionNavProvider value={go}>
      <SubpageProvider value={nav}>
        <Stack
          under={<Root onSelect={setSection} />}
          over={
            section && (
              <Pane
                section={section}
                locked={locked}
                onBack={() => {
                  if (useUi.getState().settingsLocked) return
                  nav.drop()
                  setSection(null)
                }}
              />
            )
          }
        />
      </SubpageProvider>
    </SectionNavProvider>
  )
}
