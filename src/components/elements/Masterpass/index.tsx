import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ChangeEvent,
  type KeyboardEvent
} from 'react'
import { cx } from '@/utils/cx'
import { useTranslation } from 'react-i18next'
import Error from '../Error'
import IconButton from '../IconButton'
import { EyeGlyph, EyeOffGlyph } from '@/components/Main/icons'
import type { BiometryType } from '@/api/types'
import { biometryLabel } from '@/lib/biometry'
import BiometryGlyph from '../BiometryGlyph'
import { verbatimInput } from '../inputProps'

interface Props {
  // Rendered under the card, in the field's own red. The lock screen says its
  // piece in the eyebrow instead and passes `invalid`.
  error?: string | null
  /** Draw the biometric end segment: a key is enrolled and usable right now. */
  biometric?: boolean
  // Which gate the end segment names, from the backend (see lib/biometry). The
  // fingerprint is the default because it is the only kind every non-Apple
  // platform has, and what this card said before the type was asked for.
  biometry?: BiometryType
  // The field genuinely must not accept input (lockout countdown); dims it.
  disabled?: boolean
  placeholder?: string
  testid?: string
  // The field takes focus on mount, which is right for every screen that draws
  // one. The setup screen draws two, and only the first of them should.
  autoFocus?: boolean
  // Paint the card as invalid without rendering an inline error, for the
  // screens that surface the message elsewhere: the lock screen in its
  // eyebrow, the setup screen in its strength line.
  invalid?: boolean
  // Hold the card on the success tint while the unlock lands.
  success?: boolean
  // The submitted passphrase is being verified (key derivation is deliberately
  // slow) — shimmer the passphrase and orbit the halo.
  pending?: boolean
  onEnter?: (value: string) => void
  onChange?: (event: ChangeEvent<HTMLInputElement>) => void
  onBiometric?: () => void
}

// THE master-passphrase field, one presentation everywhere it is asked for —
// the lock screen, and the first run's create and restore screens — so the
// field the user meets on day one is the one they meet every day after.
//
// The text is the input's own, so the caret, selection, scrolling and every
// way of editing are the browser's: a passphrase longer than the card scrolls
// inside it like any other field. Masked, the input is set in the mask font,
// which draws the brand asterisk where the browser asks for a bullet; revealed,
// in the mono tier. Both are tracked to one pitch (see `tracking-masked`), so
// the eye swaps each asterisk for its character where it stands.
//
// Deliberately unlike every other field in the app: a white card set gently
// into the window ground (see --lockfield-shadow) with its centered asterisks,
// whose border + halo carry the state — its own blue on focus (`lockfield`
// tokens, not the user's accent: this control's states are its character),
// red (plus a shake) on a bad passphrase, green while a successful unlock
// lands — while focus also cuts a touch deeper. Its only chrome is Touch ID (primary, when offered) and
// reveal (secondary, only once there is something to reveal) on the right
// edge. It fills its container: the caller decides how wide a card reads
// right on its screen.
export default function Masterpass({
  error,
  biometric,
  biometry = 'touch',
  disabled,
  placeholder,
  testid,
  autoFocus = true,
  invalid,
  success,
  pending,
  onEnter,
  onChange,
  onBiometric
}: Props) {
  const { t } = useTranslation()
  const [value, setValue] = useState('')
  const [reveal, setReveal] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)
  // Whether focus is anywhere in the card (the input, or one of its buttons).
  // A ref, not state: it is only ever read at the moment the card goes inert.
  const within = useRef(false)
  const heldFocus = useRef(false)

  const bad = !!error || !!invalid
  // The input can't accept keystrokes while locked out, verifying, or during
  // the success hold; only the lockout also dims the card.
  const inert = !!disabled || !!success || !!pending

  // Disabling the input (verifying / lockout) blurs it; hand focus back the
  // moment it re-enables so a failed attempt can be retyped immediately. Only
  // to the card that held focus when it went inert: two of these on one
  // screen (the setup's password and its confirmation) go inert together, and
  // the one the user was not typing in must not take over. The snapshot is a
  // layout effect so it is read before any blur the disabling causes lands.
  useLayoutEffect(() => {
    if (inert) heldFocus.current = within.current
  }, [inert])
  useEffect(() => {
    if (!inert && heldFocus.current && document.activeElement !== inputRef.current)
      inputRef.current?.focus()
  }, [inert])

  const doSubmit = (val: string) => {
    if (inert || val === '') return
    onEnter?.(val)
  }

  const handleChange = (event: ChangeEvent<HTMLInputElement>) => {
    setValue(event.currentTarget.value)
    onChange?.(event)
  }

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') doSubmit(event.currentTarget.value)
  }

  return (
    <div className="w-full">
      <div
        // Focus events bubble in React, so the card hears about its buttons too.
        onFocus={() => (within.current = true)}
        onBlur={event => {
          if (!event.currentTarget.contains(event.relatedTarget)) within.current = false
        }}
        className={cx(
          // In-card icon buttons (reveal, Touch ID) share a softened hover
          // wash; the ! outranks IconButton's own hover:bg-hover, in this one
          // place instead of at every button.
          'relative flex h-12 items-stretch rounded-xl border bg-detail transition-all duration-300 [&_button:hover]:bg-hover/60!',
          // The inset shadow is the cut; focus goes a touch deeper. It and the
          // state halo (ring) are both box-shadow, so Tailwind composes them.
          'shadow-lockfield focus-within:shadow-lockfield-deep',
          bad
            ? 'border-bad/60 ring-4 ring-bad/10 animate-[nudge_420ms_ease_both]'
            : success
              ? 'border-good/50 ring-4 ring-good/15'
              : pending
                ? 'border-lockfield-line'
                : 'border-text/12 focus-within:border-lockfield-line focus-within:ring-4 focus-within:ring-lockfield-soft',
          disabled && 'opacity-60'
        )}
      >
        {/* While verifying, a soft comet of light orbits the border (the
            orbit-ring utility owns the whole effect). */}
        {pending && (
          <span
            aria-hidden
            className="orbit-ring pointer-events-none absolute -inset-px rounded-xl"
          />
        )}
        <div className="relative flex-1">
          <input
            ref={inputRef}
            type={reveal ? 'text' : 'password'}
            className={cx(
              // The side padding keeps the text clear of the reveal button, and
              // is mirrored on the left so the row stays centered in the card.
              // An engine dims a disabled input on its own; the card decides
              // that here.
              'absolute inset-0 w-full rounded-xl border-0 bg-transparent px-10 text-center text-md caret-lockfield outline-none selection:bg-lockfield-soft disabled:opacity-100',
              'placeholder:font-sans placeholder:tracking-secret placeholder:text-text2',
              reveal ? 'font-mono tracking-revealed' : 'font-mask tracking-masked',
              // The shimmer's gradient is the ink, so it and a solid ink are
              // exclusive (see Eyebrow).
              pending ? 'shimmer-text' : reveal ? 'text-text' : 'text-text/75'
            )}
            placeholder={placeholder || t('Master Password')}
            // The passphrase is stored exactly as typed, including while
            // revealed — when `type` is `text` and nothing else would hold the
            // OS off.
            {...verbatimInput}
            disabled={inert}
            data-testid={testid}
            autoFocus={autoFocus}
            value={value}
            onChange={handleChange}
            onKeyDown={handleKeyDown}
          />
          {/* Reveal is a secondary modifier of what you're typing, so it only
              appears once there is something to reveal, small and dim. The
              crossed eye alone carries the on state — no persistent wash. */}
          {value.length > 0 && (
            <IconButton
              label={t(reveal ? 'Hide passphrase' : 'Reveal passphrase')}
              className="animate-fade absolute right-1.5 top-1/2 -translate-y-1/2"
              muted
              onClick={() => setReveal(r => !r)}
            >
              {reveal ? <EyeOffGlyph size={15} /> : <EyeGlyph size={15} />}
            </IconButton>
          )}
        </div>

        {/* Biometrics are the card's own end segment: a taller divider than the
            reveal tier, and the glyph in the macOS Touch ID rose, sized to
            nearly fill the 28px button. Which glyph and name — Touch ID or Face
            ID — is `lib/biometry`'s call, from what the device reports. */}
        {biometric && (
          <>
            <span aria-hidden className="my-auto h-7 w-px bg-line" />
            <IconButton
              label={t(biometryLabel(biometry))}
              className="mx-1.5 my-auto"
              onClick={onBiometric}
            >
              {/* Child span so the rose survives IconButton's hover ink. */}
              <span className="text-touchid">
                <BiometryGlyph type={biometry} size={22} />
              </span>
            </IconButton>
          </>
        )}
      </div>
      <Error error={error} />
    </div>
  )
}
