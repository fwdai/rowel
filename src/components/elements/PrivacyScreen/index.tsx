import Logo from '@/assets/images/logo.svg?react'
import { useObscured } from './useObscured'

/**
 * An opaque cover over the whole window for as long as the vault is on screen
 * but the user is not looking at it.
 *
 * iOS snapshots the webview for the app switcher the moment the scene resigns
 * active, and keeps that image on disk until the app is foregrounded again — so
 * a revealed password, an OTP or a note would otherwise sit in the switcher, and
 * in the snapshot file, for as long as the app is away. The same image is what
 * macOS shows in Mission Control and what a screenshot of an unfocused window
 * captures.
 *
 * Deliberately on every platform but Android (see `Main` for why not there),
 * and deliberately with no animation: a fade
 * would hand the snapshot a half-transparent frame, which is the one frame that
 * must not be taken. Mounted with the unlocked vault (`Main`), so the lock
 * screen — which has nothing to hide — never draws it.
 */
export default function PrivacyScreen() {
  const obscured = useObscured()

  if (!obscured) return null

  return (
    <div
      data-testid="privacy-screen"
      aria-hidden
      className="fixed inset-0 z-[2000] grid place-items-center bg-app"
    >
      <Logo width={72} height={72} className="fill-brand" />
    </div>
  )
}
