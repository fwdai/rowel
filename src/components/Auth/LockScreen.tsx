import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import AuthShell from '@/components/elements/AuthShell'
import Eyebrow from '@/components/elements/Eyebrow'
import Masterpass from '@/components/elements/Masterpass'
import type { BiometryType } from '@/api/types'
import { useVisualViewport, viewportStyle } from '@/hooks/useVisualViewport'
import BiometricTile from './BiometricTile'
import Brand from './Brand'
import { useUnlock } from './useUnlock'
import WorkspacePicker from './WorkspacePicker'

interface Props {
  /** Whether biometric unlock is enrolled *and* usable (see `appSlice`). */
  biometric: boolean
  /** Which gate the tile and the card's end segment name (see lib/biometry). */
  biometry?: BiometryType
}

/**
 * The phone lock screen. Same parts as the wide one and the same `useUnlock`,
 * laid out in three bands, each element where its purpose puts it:
 *
 * - The top band is who this is and which vault: the brand, and the vault
 *   chip when there is more than one. It hangs under the safe area, as a
 *   masthead does, and is not what the thumb is here for.
 * - The centre band is the one thing to do, at the exact centre of the
 *   screen: the status line, and under it the biometric tile when a key is
 *   enrolled — one tap, where the eye lands — or the passphrase card. The two
 *   bands around it are equal, so it is centred on the screen and not merely
 *   in what the masthead leaves over.
 * - The bottom band is the way round the tile: entering the passphrase, as
 *   a plain text control by the footer. A backup for the day Face ID does not
 *   answer, not a second offer of the same size as the first — a bordered
 *   52px button there read as one.
 *
 * Without an enrollment there is nothing to lead with, so the card takes the
 * centre straight away. The ground is sized to the visual viewport, so the
 * keyboard the card brings up shortens the screen under the card rather than
 * covering it — and the masthead steps down to 64px while the card is up, so
 * the top band still has room to match the bottom one and the card keeps the
 * centre of what the keyboard leaves. A band never shrinks under its content,
 * so a screen too short for even that scrolls rather than clips.
 */
export default function LockScreen({ biometric, biometry = 'touch' }: Props) {
  const { t } = useTranslation()
  const { mascot, eyebrow, field, submit, biometric: unlock, change } = useUnlock()
  const viewport = useVisualViewport()
  // Derived, not seeded: `biometric` only becomes true once the launch probe
  // answers, which is after this mounts — a card seeded from the first render
  // would never give way to the tile.
  const [revealed, setRevealed] = useState(false)
  const password = revealed || !biometric

  // ...but that probe can also land *between* two keystrokes, and swapping the
  // card for the tile then would throw away a passphrase already being typed.
  // So any use of the card is itself a decision to keep it, whatever the probe
  // says afterwards.
  const keep = () => setRevealed(true)

  return (
    <AuthShell footer fill style={viewportStyle(viewport)}>
      <div className="grid flex-1 grid-rows-[1fr_auto_1fr]">
        <div className="flex flex-col items-center">
          <Brand
            state={mascot.state}
            gaze={mascot.gaze}
            size={password ? 64 : 96}
            className={password ? 'mb-5' : 'mb-9'}
          />
          <WorkspacePicker busy={field.pending || field.success} />
        </div>

        <div className="flex flex-col items-center">
          <Eyebrow tone={eyebrow.tone} busy={eyebrow.busy} testid={eyebrow.testid}>
            {eyebrow.text}
          </Eyebrow>
          {password ? (
            <div className="mt-5 w-full max-w-[380px]">
              {/* The card keeps its own biometric segment, so revealing the
                  passphrase never takes the faster way out away. */}
              <Masterpass
                key={field.vault}
                biometric={biometric}
                biometry={biometry}
                testid="unlock-password-input"
                invalid={field.invalid}
                success={field.success}
                pending={field.pending}
                disabled={field.disabled}
                onChange={event => {
                  keep()
                  change(event)
                }}
                onEnter={value => {
                  keep()
                  submit(value)
                }}
                onBiometric={unlock}
              />
            </div>
          ) : (
            <div className="mt-5">
              <BiometricTile biometry={biometry} onUnlock={unlock} />
            </div>
          )}
        </div>

        <div className="flex items-end justify-center">
          {!password && (
            // The phone's text-control tier: 44px tall in the accent ink, as
            // the nav rows' Cancel and Save are, with no box to weigh it.
            <button
              type="button"
              data-testid="use-password-button"
              onClick={() => setRevealed(true)}
              className="mb-2 flex h-11 cursor-pointer items-center rounded-lg px-4 text-md font-medium text-accent transition-colors active:bg-hover"
            >
              {t('Enter Master Password')}
            </button>
          )}
        </div>
      </div>
    </AuthShell>
  )
}
