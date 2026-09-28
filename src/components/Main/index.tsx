import { useLayout } from '@/hooks/useLayout'
import { useActivityPing } from '@/hooks/useActivityPing'
import { FrameProvider } from '@/components/elements/Frame'
import Modal from '@/components/elements/Modal'
import Sheet from '@/components/elements/Sheet'
import CopyToast from '@/components/elements/CopyToast'
import NoticeToast from '@/components/elements/NoticeToast'
import PrivacyScreen from '@/components/elements/PrivacyScreen'
import Wide from './Wide'
import Compact from './Compact'
import Palette from './Palette'
import AddSecret from './AddSecret'
import Share from './Share'
import Scan from './Scan'
import EnvDrop from './EnvDrop'
import BrowserConsent from './BrowserConsent'
import { useShortcuts } from './useShortcuts'

// Two shells, one set of overlays. `Wide` is the three-pane desktop/iPad layout
// (rail 56px · list 348px · detail flex); `Compact` is the phone one, a screen
// at a time over a tab bar. This is the one place that asks which: everything
// below is either shared, composed differently by the two shells, or — for the
// overlays — framed by the `Frame` handed down from here.
export function Main() {
  const compact = useLayout() === 'compact'
  useShortcuts()
  // The unlocked vault is the only place there is an idle clock to feed.
  useActivityPing()

  return (
    <div
      data-testid="main-view"
      className="flex h-full flex-col overflow-hidden bg-app font-sans text-text select-none"
    >
      <FrameProvider value={compact ? Sheet : Modal}>
        {/* The generator is not here: it is a dialog on one shell and a tab
            root plus an overlay on the other, so each shell mounts its own. */}
        {compact ? <Compact /> : <Wide />}
        {/* ⌘K needs a keyboard to reach and a rail's worth of room to read: on a
            phone it is neither reachable nor the way anything is found. */}
        {!compact && <Palette />}
        <AddSecret />
        <Share />
        <Scan />
        <EnvDrop />
        <BrowserConsent />
      </FrameProvider>
      <CopyToast />
      <NoticeToast />
      {/* Over everything, including the overlays above — and only here, because
          the unlocked vault is the only screen with secrets on it to keep out
          of the app switcher. */}
      <PrivacyScreen />
    </div>
  )
}

export default Main
