import { useEffect, useRef, useState, type ChangeEvent } from 'react'
import { useTranslation } from 'react-i18next'
import type { TFunction } from 'i18next'
import type { UnlockResult } from '@/api/types'
import { unlock, unlockBiometric } from '@/api/auth'
import { workspaceAdoptVaultPassword } from '@/api/workspace'
import { errorKind, isTooManyAttempts, vaultPasswordRequiredFor } from '@/api/errors'
import { enterMain, useApp, selectActiveWorkspace } from '@/store'
import type { MascotState } from '@/components/elements/Mascot'
import { unsealError } from '@/components/Start/shared/errors'
import { untilFocused } from '@/utils/untilFocused'

// How long the mascot gets to celebrate before the vault fades in.
const SUCCESS_HOLD_MS = 650

// How long, past the hold, the unlock waits for the window to be in front.
// On iOS the Face ID sheet leaves the scene inactive while it comes down, and
// a vault that mounts behind it is asked to draw its privacy cover on first
// sight — a flash the user reads as the app breaking. Entering only once the
// window says it is focused keeps the cover down on the way in. Bounded: a
// user who really did swipe away gets in (covered) rather than waiting forever.
const FOCUS_WAIT_MS = 3000

// The password is blamed only when the backend said `invalidPassword`. An open
// can also fail on I/O, a stuck lock or a corrupt file — none of which counted
// against the lockout — and those keep their own description.
const unlockError = (t: TFunction, error: unknown): string =>
  unsealError(t, error, t('Incorrect Master Password'))

// A biometric failure is never a password problem: the backend's errors here
// are a cancelled/failed prompt, a missing enrollment, or a keychain issue.
// Claiming "Incorrect Master Password" for any of them would send the user
// retyping a password that was never checked.
const biometricError = (t: TFunction, error: unknown): string =>
  errorKind(error) === 'vaultTooNew'
    ? t('Vault needs a newer version of the app')
    : t('Biometric unlock failed')

const lockedMessage = (t: TFunction, seconds: number) =>
  t('Too many failed attempts. Try again in {{seconds}}s', { seconds })

// One attempt-lifecycle slot instead of separate success/pending booleans:
// only one of these can be true at a time, and everything below derives from
// the same precedence.
type Phase = 'idle' | 'verifying' | 'success'

// A lockbox that opened with a password of its own, which is not the vault
// password: it stays sealed (`vaultPasswordRequired`), and the next passphrase
// the card takes is the vault password, which moves it over and then opens it.
interface Moving {
  workspaceId: string
  lockboxPassword: string
}

export interface Unlock {
  mascot: { state: MascotState; gaze: number }
  /** The status line: one element carries all three states, named by testid. */
  eyebrow: {
    text: string
    tone: 'muted' | 'bad'
    busy: boolean
    testid: string
  }
  /** How the passphrase card presents the attempt. */
  field: {
    invalid: boolean
    success: boolean
    pending: boolean
    disabled: boolean
    /**
     * What the passphrase is for: the workspace, and whether the card is now
     * asking for the vault password. The card is keyed on it, so a switch — or
     * the move to the vault password — hands it an empty field rather than the
     * passphrase already typed for the last question.
     */
    vault: string
    /** The card's placeholder, when it is asking for something else. */
    placeholder?: string
  }
  submit: (value: string) => void
  biometric: () => void
  change: (event: ChangeEvent<HTMLInputElement>) => void
}

/**
 * One unlock attempt, in whichever shell is drawing it. Every piece of state a
 * lock screen has lives here — the attempt phase, the lockout countdown, the
 * gaze the mascot reads along with — so the wide card and the phone's
 * biometric-first screen are the same behaviour laid out two ways.
 */
export function useUnlock(): Unlock {
  const { t } = useTranslation()
  const [error, setError] = useState<string | null>(null)
  const [retryAfter, setRetryAfter] = useState(0)
  const [count, setCount] = useState(0)
  const [phase, setPhase] = useState<Phase>('idle')
  const [moving, setMoving] = useState<Moving | null>(null)
  const holdTimer = useRef(0)
  // The unlock the hold is sitting on. The backend has already opened the vault
  // by the time this is set, so it must reach `enterMain` whatever happens to
  // this hook — see the unmount cleanup below.
  const pending = useRef<UnlockResult | null>(null)
  const vault = useApp(selectActiveWorkspace)
  const switching = useApp(state => state.switching)

  // A new vault on screen starts from nothing typed and nothing wrong: what
  // the mascot read along with, and the error, were about the last one. Reset
  // in render rather than in an effect, so the frame the vault changes in
  // already draws it clean.
  const [shown, setShown] = useState(vault)
  if (shown !== vault) {
    setShown(vault)
    setCount(0)
    setMoving(null)
    if (retryAfter <= 0) setError(null)
  }

  // Countdown ticks once a second while locked out; re-enables the input at 0.
  useEffect(() => {
    if (retryAfter <= 0) return
    const id = setTimeout(() => {
      const next = retryAfter - 1
      setRetryAfter(next)
      setError(next > 0 ? lockedMessage(t, next) : null)
    }, 1000)
    return () => clearTimeout(id)
  }, [retryAfter, t])

  // An unmount mid-hold hands the unlock on at once rather than dropping it.
  // The wide and compact lock screens are different components, so a window
  // resized across the layout breakpoint during the hold remounts this hook —
  // and dropping the timer with it left Rust unlocked behind a UI still asking
  // for the password.
  //
  // Only while the flow is still `auth`, which is what a layout remount leaves
  // it on. An unmount because the flow itself moved on — a workspace create
  // landing its own unlock, a return to setup — means someone else owns the
  // session now, and this result describes a vault that is no longer open.
  useEffect(
    () => () => {
      clearTimeout(holdTimer.current)
      const result = pending.current
      pending.current = null
      if (result && useApp.getState().flow === 'auth') void enterMain(result)
    },
    []
  )

  // Let the mascot celebrate, then wait for the window to be in front, before
  // the vault takes over. Whoever clears `pending` first — this, or the
  // unmount above — is the one that enters.
  const holdThenEnter = (result: UnlockResult) => {
    setPhase('success')
    pending.current = result
    holdTimer.current = window.setTimeout(() => {
      void untilFocused(FOCUS_WAIT_MS).then(() => {
        if (pending.current !== result) return
        pending.current = null
        void enterMain(result)
      })
    }, SUCCESS_HOLD_MS)
  }

  const handleEnter = (value: string) => {
    // Mid-switch the backend is already pointed at a vault the screen does not
    // show yet; the attempt would go there.
    if (retryAfter > 0 || phase !== 'idle' || switching) return
    // Key derivation is deliberately slow; acknowledge the Enter immediately
    // (and drop any stale error — this attempt owns the eyebrow now).
    setError(null)
    setPhase('verifying')
    // Asking for the vault password: move the lockbox onto it, then open it
    // with it — the one password every lockbox opens with from here on.
    const attempt = moving
      ? workspaceAdoptVaultPassword(moving.workspaceId, moving.lockboxPassword, value).then(() =>
          unlock(value)
        )
      : unlock(value)
    attempt
      .then(result => {
        setMoving(null)
        holdThenEnter(result)
      })
      .catch((err: unknown) => {
        setPhase('idle')
        const workspaceId = vaultPasswordRequiredFor(err)
        if (workspaceId) {
          // The card is keyed on the question, so it comes back empty.
          setCount(0)
          setMoving({ workspaceId, lockboxPassword: value })
        } else if (isTooManyAttempts(err)) {
          setRetryAfter(err.retryAfterSecs)
          setError(lockedMessage(t, err.retryAfterSecs))
        } else {
          setError(unlockError(t, err))
        }
      })
  }

  const handleTouchId = () => {
    // Biometric unlock is never subject to the password backoff (the OS gate
    // already rate-limits it), so it stays available even while locked out.
    if (phase !== 'idle' || switching) return
    setError(null)
    setPhase('verifying')
    unlockBiometric()
      .then(holdThenEnter)
      .catch((err: unknown) => {
        setPhase('idle')
        // A prompt the user dismissed is their choice, not a failure to
        // report in red: the screen goes back to waiting, passphrase at hand.
        if (errorKind(err) !== 'cancelled') setError(biometricError(t, err))
      })
  }

  const handleChange = (event: ChangeEvent<HTMLInputElement>) => {
    setCount(event.currentTarget.value.length)
    if (retryAfter <= 0) setError(null)
  }

  // The mascot reads along as you type: the gaze pans left-to-right with the
  // caret (16 chars ≈ full sweep, like the prototype; Mascot clamps to ±1).
  const gaze = count > 0 ? (count / 16) * 2 - 1 : 0
  const state: MascotState =
    phase !== 'idle'
      ? phase === 'success'
        ? 'success'
        : 'checking'
      : error
        ? 'error'
        : count > 0
          ? 'typing'
          : 'idle'

  return {
    mascot: { state, gaze },
    eyebrow: {
      text:
        error ??
        (phase === 'success'
          ? t('Unsealing')
          : phase === 'verifying'
            ? t('Verifying')
            : moving
              ? t(
                  'Rowel opens every lockbox with your vault password. Enter it to move this lockbox over.'
                )
              : t('Vault sealed')),
      tone: error ? 'bad' : 'muted',
      busy: phase === 'verifying',
      // The testid names which of the three states is showing rather than
      // forcing specs to parse the message.
      testid:
        retryAfter > 0 ? 'unlock-lockout' : error ? 'unlock-error' : 'unlock-status'
    },
    field: {
      invalid: !!error,
      success: phase === 'success',
      pending: phase === 'verifying',
      disabled: retryAfter > 0,
      vault: moving ? `${vault}:vault-password` : vault,
      placeholder: moving ? t('Vault password') : undefined
    },
    submit: handleEnter,
    biometric: handleTouchId,
    change: handleChange
  }
}
