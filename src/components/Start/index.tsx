import { useCallback, useEffect, useRef, useState } from 'react'
import type { BiometryType, UnlockResult } from '@/api/types'
import { setupCreate } from '@/api/setup'
import type { Provider } from '@/api/provider'
import { claimOpenedFile, connectAccount, enterMain, forgetAccount, useApp } from '@/store'
import { isRestorableFile } from '@/lib/backup'
import AuthShell from '@/components/elements/AuthShell'
import Mascot, { type MascotState } from '@/components/elements/Mascot'
import BiometryGlyph from '@/components/elements/BiometryGlyph'
import Welcome from './Welcome'
import CloudPicker from './CloudPicker'
import Password from './Password'
import Sync from './Sync'
import Conflict from './Conflict'
import Account from './Account'
import File from './File'
import Biometric from './Biometric'

type Screen =
  | 'welcome'
  | 'cloud'
  | 'password'
  | 'sync'
  | 'conflict'
  | 'account'
  | 'file'
  | 'biometric'

// How far along each screen is, as the mascot's smile tells it: neutral on
// the welcome, brightening through the middle, all the way once the vault is
// about to exist. The restore path sits half way, as the password step does;
// picking a cloud is a step towards it, not yet on it.
const JOY: Record<Screen, number> = {
  welcome: 0,
  cloud: 0.25,
  password: 0.5,
  sync: 1,
  conflict: 0.5,
  account: 0.5,
  file: 0.5,
  biometric: 1
}

/**
 * The first run, end to end.
 *
 * Screens are a stack rather than a graph: every one of them is reached from
 * exactly one place, so "back" is "pop" and no screen has to know who sent it.
 * The two screens with nothing behind them — the welcome, and the biometric
 * question once the session is already open — are the two that draw no Back,
 * which falls out of the stack being one deep rather than being decided again
 * per screen.
 *
 * The shell, the Back button and the mascot are drawn here, once, and stay
 * mounted from screen to screen: only the content under the mascot changes,
 * and it slides in from the side it came from — the right going forward, the
 * left coming back — while the mascot's smile eases to the new step's level.
 * Each screen is only its content.
 *
 * Nothing here enters the app until the very end: unlocking is not the last
 * step, enrolling a fingerprint is, and that can only be asked of an open
 * session.
 */
export function Start() {
  const [stack, setStack] = useState<Screen[]>(['welcome'])
  const screen = stack[stack.length - 1]
  // Which way the last move went, so the arriving content knows which side
  // to come in from.
  const [direction, setDirection] = useState<'forward' | 'back'>('forward')

  // The shell outlives the screens now, and so would its scroll position: a
  // step read to the bottom on a short screen would hand the next step over
  // part way down. Every arrival starts at the top.
  const shell = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (shell.current) shell.current.scrollTop = 0
  }, [screen])

  const [password, setPassword] = useState('')
  // A `.rowel` or a legacy `.swftx` the OS opened the app with lands straight
  // on the restore step, file already picked: either becomes this device's
  // vault from there.
  const [restoreFrom, setRestoreFrom] = useState<string | null>(null)
  const opened = useApp(state => state.openedFile)
  useEffect(() => {
    if (!opened || !isRestorableFile(opened)) return
    claimOpenedFile()
    setRestoreFrom(opened)
    setDirection('forward')
    setStack(['welcome', 'file'])
  }, [opened])
  // Consent already given, on the way through the restore screen: the create
  // flow adopts those tokens instead of asking for an account a second time.
  const [accountLinked, setAccountLinked] = useState(false)
  const [result, setResult] = useState<UnlockResult | null>(null)
  const [biometry, setBiometry] = useState<BiometryType>('touch')

  // The account probe's state colours the mascot on the screens that are
  // waiting on it: eyes narrowed while consent is out, a head shake when the
  // provider did not answer.
  const probe = useApp(state => state.setupAccount.status)
  const waiting = (screen === 'account' || screen === 'sync') && probe === 'pending'
  const failed = screen === 'account' && probe === 'error'
  const mood: MascotState = failed ? 'error' : waiting ? 'checking' : 'idle'

  const go = (next: Screen) => {
    setDirection('forward')
    setStack(current => [...current, next])
  }

  const back = () => {
    // Stepping out of the account screen towards the cloud picker gives the
    // tokens back: the user is no longer restoring from that account.
    // Stepping back into the conflict question keeps them — that screen is
    // about the very file this one was showing.
    if (screen === 'account' && stack[stack.length - 2] === 'cloud') {
      forgetAccount()
      setAccountLinked(false)
    }
    // Backing out of the conflict question is refusing both of its answers, so
    // the account goes with it — the backup step would otherwise re-read the
    // probe still sitting in the store and bounce straight back here.
    if (screen === 'conflict') forgetAccount()
    // Backing out of the backup step abandons a consent that may still be out
    // with the browser. Without this its late answer would sit in the store,
    // and the next visit to this step would read "empty" and create at once,
    // never having asked.
    if (screen === 'sync') forgetAccount()
    // Leaving the restore step is declining the file the OS opened; the next
    // visit starts from the picker.
    if (screen === 'file') setRestoreFrom(null)
    setDirection('back')
    setStack(current => (current.length > 1 ? current.slice(0, -1) : current))
  }

  /**
   * An open session, and one question left. Biometrics are only asked about
   * where they can be offered; everywhere else this is simply the way in. "Can
   * be offered" is whether enrolling would work — not `biometric.available`,
   * which also asks whether it already *has* been, and so is false on every
   * fresh install by definition.
   */
  const finish = useCallback(async (unlocked: UnlockResult) => {
    // The boot probe already answered, and `canEnroll` and `type` are facts
    // about the device that do not change during setup.
    const biometric = useApp.getState().status?.biometric
    if (!biometric?.canEnroll) return enterMain(unlocked)
    setResult(unlocked)
    setBiometry(biometric.type)
    // Replaces the stack: there is no going back from an unlocked session.
    setDirection('forward')
    setStack(['biometric'])
  }, [])

  const create = useCallback(
    (value: string) => setupCreate(value).then(finish),
    [finish]
  )

  const createWithPassword = useCallback(() => create(password), [create, password])

  const openAccount = (provider: Provider) => {
    connectAccount(provider)
    go('account')
  }

  const continueFromPassword = (value: string) => {
    setPassword(value)
    // Coming back through the restore screen, the account is already agreed:
    // there is no second step left to ask about.
    if (accountLinked) return create(value)
    go('sync')
    return Promise.resolve()
  }

  const content = () => {
    switch (screen) {
      case 'welcome':
        return (
          <Welcome
            onFresh={() => go('password')}
            onCloud={() => go('cloud')}
            onFile={() => go('file')}
          />
        )
      case 'cloud':
        return <CloudPicker onPick={openAccount} />
      case 'password':
        return <Password onContinue={continueFromPassword} />
      case 'sync':
        return <Sync onCreate={createWithPassword} onConflict={() => go('conflict')} />
      case 'conflict':
        return <Conflict onUnlockExisting={() => go('account')} />
      case 'account':
        return (
          <Account
            onStartFresh={() => {
              setAccountLinked(true)
              go('password')
            }}
            onUseFile={() => {
              // Restoring from a file is leaving this account behind: the
              // tokens it holds would otherwise outlive the flow in the
              // backend's memory.
              forgetAccount()
              setAccountLinked(false)
              go('file')
            }}
            onRestored={finish}
          />
        )
      case 'file':
        return <File onRestored={finish} initialPath={restoreFrom} />
      case 'biometric':
        return (
          <Biometric
            biometry={biometry}
            onDone={() => {
              if (result) void enterMain(result)
            }}
          />
        )
    }
  }

  return (
    <AuthShell ref={shell} onBack={stack.length > 1 ? back : undefined}>
      <div className="mb-6 flex justify-center">
        {/* The last question is about the device's own gate, so that is what
            sits above it — the mascot has seen the user through. */}
        {screen === 'biometric' ? (
          <span className="grid h-16 w-16 place-items-center rounded-xl bg-tile text-touchid">
            <BiometryGlyph type={biometry} size={30} />
          </span>
        ) : (
          <Mascot state={mood} joy={JOY[screen]} />
        )}
      </div>
      {/* Keyed on the screen: a new key is a fresh element, which is what
          runs the arrival animation. Content changing within a screen (the
          account probe answering) redraws in place. */}
      <div
        key={screen}
        className={direction === 'back' ? 'animate-step-back' : 'animate-step-forward'}
      >
        {content()}
      </div>
    </AuthShell>
  )
}

export default Start
