import Masterpass from '@/components/elements/Masterpass'
import AuthShell from '@/components/elements/AuthShell'
import Eyebrow from '@/components/elements/Eyebrow'
import type { BiometryType } from '@/api/types'
import Brand from './Brand'
import { useUnlock } from './useUnlock'
import WorkspacePicker from './WorkspacePicker'

interface Props {
  /** Whether biometric unlock is enrolled *and* usable (see `appSlice`). */
  biometric: boolean
  /** Which gate the card's end segment names (see lib/biometry). */
  biometry?: BiometryType
}

// The wide lock screen: the brand (mascot and wordmark), which vault is about
// to open, the status line and the passphrase card, centered in the auth
// ground. Biometrics live inside the card as its end segment — there is a
// keyboard here, so typing is the lead affordance. The phone leads the other
// way round (see LockScreen); both run the same `useUnlock`.
export function Auth({ biometric, biometry = 'touch' }: Props) {
  const { mascot, eyebrow, field, submit, biometric: unlock, change } = useUnlock()

  return (
    <AuthShell footer>
      <Brand state={mascot.state} gaze={mascot.gaze} />
      <WorkspacePicker busy={field.pending || field.success} />
      <Eyebrow tone={eyebrow.tone} busy={eyebrow.busy} testid={eyebrow.testid}>
        {eyebrow.text}
      </Eyebrow>
      {/* One field alone on a wide ground: capped so it reads as a card, not a
          bar. */}
      <div className="mx-auto mt-8 max-w-[380px]">
        <Masterpass
          key={field.vault}
          biometric={biometric}
          biometry={biometry}
          testid="unlock-password-input"
          placeholder={field.placeholder}
          invalid={field.invalid}
          success={field.success}
          pending={field.pending}
          disabled={field.disabled}
          onChange={change}
          onEnter={submit}
          onBiometric={unlock}
        />
      </div>
    </AuthShell>
  )
}

export default Auth
