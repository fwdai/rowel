import { useVault, useUi } from '@/store'
import { useVisualViewport, viewportStyle } from '@/hooks/useVisualViewport'
import Attached from '../Generator/Attached'
import Entry from './Entry'
import Generator from './Generator'
import Settings from './Settings'
import Vault from './Vault'
import TabBar from './TabBar'
import Stack from './Stack'

/**
 * The phone shell: a tab root, and at most one screen pushed over it, both
 * derived from the store.
 *
 * There is no router. The screens are the rows of the navigation model
 * (docs/compact-shell.md) that the store makes true — a draft or an edit is the
 * form, a surface opened over the vault is that surface, a selection is the
 * detail, otherwise a tab root. Nothing here is new state, so ⌘K, the tray and
 * a save all navigate by doing what they already do.
 *
 * The order matters in one place: the two roots that can be *opened* while a row
 * is selected (Settings, the standalone generator) come before the selection, or
 * ⌘G with an entry on screen would set `generator.open` and change nothing
 * visible. A draft still wins over all of it — it is the one screen with
 * unsaved work on it. Tabs clear the selection through `setView` anyway.
 *
 * The root stays mounted under a pushed screen (`Stack`), with its tab bar,
 * so it keeps its scroll and is there to come back to — receding under a push
 * and sliding back under an edge swipe. It is inert the while: switching view
 * from under a form would drop the draft out from under it.
 */
export default function Compact() {
  const writing = useVault(state => state.creating !== null || state.editing)
  const entry = useVault(state => state.currentId !== null)
  const settings = useUi(state => state.settings)
  // The standalone generator is a root here. Opened from a password row it
  // carries somewhere to put the value, and stays the overlay it is on the
  // desktop — that one is `Attached`, below.
  const generator = useUi(
    state => state.generator.open && !state.generator.apply && !state.generator.ssh
  )
  const viewport = useVisualViewport()
  // A root is over the selection, so the bar it is left by has to stay up.
  const pushed = writing || (entry && !settings && !generator)

  return (
    <>
      <div
        data-testid="compact-shell"
        style={viewportStyle(viewport)}
        // The ground is painted here as well as on each screen, so nothing of
        // the main root's darker `bg-app` shows between them.
        className="relative flex h-full min-h-0 flex-col bg-screen"
      >
        {/* The entry screen is one component for reading and writing, in one
            slot, so React keeps the instance — and with it the one reveal —
            across the step from one to the other. The form rises rather than
            pushes: it is a mode, not a step along a path. */}
        <Stack
          motion={writing ? 'rise' : 'push'}
          under={
            <>
              {settings ? <Settings /> : generator ? <Generator /> : <Vault />}
              {/* Pinned to the root's layer, so it goes where the root goes. */}
              <TabBar />
            </>
          }
          over={pushed && <Entry />}
        />
      </div>
      {/* Outside the shell, like every fixed overlay: the shell carries
          `viewportStyle`'s translate, which makes it the containing block of
          anything `fixed` inside it — the sheet would take the keyboard offset
          a second time. */}
      <Attached />
    </>
  )
}
