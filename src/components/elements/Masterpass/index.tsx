import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ChangeEvent,
  type KeyboardEvent,
  type MouseEvent
} from 'react'
import { cx } from '@/utils/cx'
import { useTranslation } from 'react-i18next'
import Error from '../Error'
import IconButton from '../IconButton'
import { EyeGlyph, EyeOffGlyph } from '@/components/Main/icons'
import type { BiometryType } from '@/api/types'
import { verbatimInput } from '../inputProps'
import Biometric from './Biometric'
import Dots, { CELL } from './Dots'

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
  // Take the caret back whenever the window comes to the front, for a screen
  // where this field is the one thing to type into (the lock screen).
  refocus?: boolean
  // Paint the card as invalid without rendering an inline error, for the
  // screens that surface the message elsewhere: the lock screen in its
  // eyebrow, the setup screen in its strength line.
  invalid?: boolean
  // Hold the card on the success tint while the unlock lands.
  success?: boolean
  // The submitted passphrase is being verified (key derivation is deliberately
  // slow) — ripple the dots and orbit the halo.
  pending?: boolean
  onEnter?: (value: string) => void
  onChange?: (event: ChangeEvent<HTMLInputElement>) => void
  onBiometric?: () => void
}

// THE master-passphrase field, one presentation everywhere it is asked for —
// the lock screen, and the first run's create and restore screens — so the
// field the user meets on day one is the one they meet every day after. The
// real value and selection live in the input; mirrored copies drive the dot
// overlay. Masking is done with a text-transparent input + custom dots so the
// caret and letter spacing match the design in both themes. Because the
// input's own glyph geometry never matches the fixed cell grid, clicks are
// mapped to a character index against the drawn cells rather than left to the
// browser, and the overlay caret tracks the input's real selection so it
// always shows where the next edit lands.
//
// Deliberately unlike every other field in the app: a white card set gently
// into the window ground (see --lockfield-shadow) with big centered dots,
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
  refocus,
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
  const [focused, setFocused] = useState(false)
  const [selection, setSelection] = useState<[number, number]>([0, 0])
  const inputRef = useRef<HTMLInputElement>(null)
  const rowRef = useRef<HTMLDivElement>(null)
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

  // A window brought back to the front is typed into straight away, without a
  // click to find the field first. Focus the user left somewhere that takes
  // keys of its own stays there: another field, or an open menu (the vault
  // picker, whose arrows and Enter would otherwise land here).
  useEffect(() => {
    if (!refocus) return
    const onWindowFocus = () => {
      const input = inputRef.current
      const active = document.activeElement
      const elsewhere =
        active !== input &&
        (active instanceof HTMLInputElement || !!active?.closest('[role="menu"]'))
      if (!elsewhere) input?.focus()
    }
    window.addEventListener('focus', onWindowFocus)
    return () => window.removeEventListener('focus', onWindowFocus)
  }, [refocus])

  // With nothing typed there is no passphrase to submit, so Enter asks the
  // biometric gate instead, when the card offers one.
  const doSubmit = (val: string) => {
    if (inert) return
    if (val !== '') onEnter?.(val)
    else if (biometric) onBiometric?.()
  }

  // Mirror the input's selection into state so the overlay caret follows it.
  // Called from every event that can move it (typing, arrows, select-all,
  // focus, and our own click mapping).
  const syncSelection = () => {
    const el = inputRef.current
    if (!el) return
    const end = el.value.length
    setSelection([el.selectionStart ?? end, el.selectionEnd ?? end])
  }

  const handleChange = (event: ChangeEvent<HTMLInputElement>) => {
    setValue(event.currentTarget.value)
    syncSelection()
    onChange?.(event)
  }

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    // A held Enter is one press: its repeats must not ask the biometric gate
    // of a field the first one just emptied.
    if (event.key === 'Enter' && !event.repeat) doSubmit(event.currentTarget.value)
  }

  // Place the caret on the cell boundary nearest the click. The browser would
  // otherwise position it by the input's own (transparent, differently spaced)
  // glyphs, landing it on a different character than the one under the pointer.
  const handleMouseDown = (event: MouseEvent<HTMLInputElement>) => {
    const row = rowRef.current
    if (event.button !== 0 || !row) return
    event.preventDefault()
    const input = event.currentTarget
    const x = event.clientX - row.getBoundingClientRect().left
    const at = Math.max(0, Math.min(input.value.length, Math.round(x / CELL)))
    input.focus()
    input.setSelectionRange(at, at)
    syncSelection()
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
            // The input's own text never shows: masked dots and the revealed
            // value are both drawn by the cell overlay (see Dots) so they share
            // one geometry, including the selection wash. Only the placeholder
            // renders from here (15px, muted ink).
            className="absolute inset-0 w-full rounded-xl border-0 bg-transparent px-10 text-center font-sans text-md tracking-secret text-transparent caret-transparent outline-none selection:bg-transparent placeholder:text-text2"
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
            onKeyUp={syncSelection}
            onSelect={syncSelection}
            onMouseDown={handleMouseDown}
            onFocus={() => {
              setFocused(true)
              syncSelection()
            }}
            onBlur={() => setFocused(false)}
          />
          <Dots
            count={value.length}
            caret={focused && !inert}
            selection={selection}
            rowRef={rowRef}
            text={reveal ? value : undefined}
            busy={pending}
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

        {biometric && (
          <Biometric biometry={biometry} enter={value === ''} onClick={onBiometric} />
        )}
      </div>
      <Error error={error} />
    </div>
  )
}
