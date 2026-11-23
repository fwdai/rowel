import { useId, useState, type KeyboardEvent, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { describeError, vaultPasswordRequiredFor } from '@/api/errors'
import { moveRestoredLockbox, switchWorkspace } from '@/store'
import Button from '../Button'
import Field from '../Field'
import Note from '../Note'
import { inputClass } from '../formStyles'
import { unsealError } from '@/components/Start/shared/errors'
import SecretField from './SecretField'

interface Props {
  /** What is being restored, drawn above the fields: a vault to pick, a file. */
  children: ReactNode
  /**
   * What Restore restores — an account's pack, a backup file's path — or
   * `null` while there is nothing to restore yet. A new one starts the
   * vault-password step over: that step belonged to what was restored before.
   */
  target: string | null
  /** A restore is in flight; every control stands down. */
  busy: boolean
  /**
   * Restore `target` as a workspace called `name`, unlocked with `password`.
   * An empty `name` leaves the naming to the backend, which takes the one the
   * vault carries in its pack. A rejection is shown under the password field —
   * a wrong one as `wrongPassword`.
   */
  onRestore: (name: string, password: string, target: string) => Promise<void>
  /** What a wrong password is told, which depends on where the pack came from. */
  wrongPassword: string
  /** A second action beside Restore, drawn while nothing runs or is being moved. */
  aside?: ReactNode
  /** Test id base for the form's controls. */
  testid?: string
}

/**
 * The lockbox a restore installed closed, because the password that opened it
 * is not the vault password, and that password — the one the move proves it
 * with. Once `moved`, that password opens nothing any more and is dropped: the
 * move cannot be taken back, so a switch that fails after it is retried on its
 * own and never sent back to the move.
 */
type Moving =
  | { workspaceId: string; moved: false; lockboxPassword: string }
  | { workspaceId: string; moved: true }

/**
 * Name, password, Restore: the last step of every "make a workspace out of a
 * vault" flow, wherever the vault came from — an account, or a backup file.
 * What is being restored is the caller's to draw and to pick — it outlives a
 * wrong password — so this only draws it above the fields.
 *
 * The name is optional: a vault carries its own, given wherever it was made,
 * and most restores want exactly that one. Typing one renames it everywhere.
 *
 * A vault sealed under a password other than the vault password is restored
 * closed (`vaultPasswordRequired`). The form then asks for the vault password
 * as well, and Restore moves the lockbox onto it and opens it: every lockbox
 * opens with the one password. A switch that fails once the move has gone
 * through (a sync holding the vault, say) is offered again on its own.
 *
 * Plain labelled fields, like every other form in Settings: the lock screen's
 * card is for the lock screen.
 */
export default function RestoreForm({
  children,
  target,
  busy,
  onRestore,
  wrongPassword,
  aside,
  testid = 'workspace-restore'
}: Props) {
  const { t } = useTranslation()
  const id = useId()
  const [name, setName] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [moving, setMoving] = useState<Moving | null>(null)
  const [vaultPassword, setVaultPassword] = useState('')
  const [vaultError, setVaultError] = useState<string | null>(null)
  const [openError, setOpenError] = useState<string | null>(null)
  const [movingBusy, setMovingBusy] = useState(false)
  const working = busy || movingBusy

  // Another pick is another restore: whatever the last one left half-moved
  // stays installed (and, once moved, opens from the list like any other), but
  // this form starts over. Reset in render, so the frame it changes in is clean.
  const [picked, setPicked] = useState(target)
  if (picked !== target) {
    setPicked(target)
    setMoving(null)
    setVaultPassword('')
    setVaultError(null)
    setOpenError(null)
  }

  const restore = () => {
    if (busy || !target) return
    setError(null)
    const typed = password
    onRestore(name.trim(), typed, target).catch((err: unknown) => {
      const workspaceId = vaultPasswordRequiredFor(err)
      if (workspaceId) {
        setMoving({ workspaceId, moved: false, lockboxPassword: typed })
        return
      }
      setError(unsealError(t, err, wrongPassword))
    })
  }

  // The switch every restore ends in, on its own: the step retried after a
  // move that went through.
  const open = (workspaceId: string) => {
    setOpenError(null)
    setMovingBusy(true)
    switchWorkspace(workspaceId)
      .catch((err: unknown) => setOpenError(describeError(err) || t('Something went wrong')))
      .finally(() => setMovingBusy(false))
  }

  const move = () => {
    if (!moving || working) return
    if (moving.moved) {
      open(moving.workspaceId)
      return
    }
    if (!vaultPassword) return
    const { workspaceId } = moving
    setVaultError(null)
    setMovingBusy(true)
    moveRestoredLockbox(workspaceId, moving.lockboxPassword, vaultPassword).then(
      () => {
        // Moved for good: neither password is asked for again.
        setMoving({ workspaceId, moved: true })
        setPassword('')
        setVaultPassword('')
        open(workspaceId)
      },
      (err: unknown) => {
        setVaultError(unsealError(t, err, t('Incorrect Master Password')))
        setMovingBusy(false)
      }
    )
  }

  const submit = moving ? move : restore

  // Not while an IME is composing: that Enter confirms the characters, and a
  // restore started on it would carry a half-typed name across devices.
  const onEnter = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter' && !event.nativeEvent.isComposing) submit()
  }

  return (
    <div className="flex max-w-sm flex-col gap-3">
      {children}
      <Field id={`${id}-name`} label={t('Lockbox name (optional)')}>
        <input
          id={`${id}-name`}
          type="text"
          className={inputClass}
          data-testid={`${testid}-name`}
          value={name}
          disabled={working || !!moving}
          onChange={event => setName(event.target.value)}
          onKeyDown={onEnter}
        />
      </Field>
      <SecretField
        id={`${id}-password`}
        label={t('Master password')}
        value={password}
        disabled={working || !!moving}
        error={error}
        testid={`${testid}-password`}
        onChange={value => {
          setError(null)
          setPassword(value)
        }}
        onKeyDown={onEnter}
      />
      {moving && !moving.moved && (
        <>
          <Note testid={`${testid}-vault-password-why`}>
            {t(
              'Rowel opens every lockbox with your vault password. Enter it to move this lockbox over.'
            )}
          </Note>
          <SecretField
            id={`${id}-vault-password`}
            label={t('Vault password')}
            value={vaultPassword}
            disabled={working}
            error={vaultError}
            testid={`${testid}-vault-password`}
            onChange={value => {
              setVaultError(null)
              setVaultPassword(value)
            }}
            onKeyDown={onEnter}
          />
        </>
      )}
      {openError && (
        <Note tone="bad" testid={`${testid}-open-error`}>
          {openError}
        </Note>
      )}
      <div className="flex items-center gap-3">
        <Button testid={`${testid}-submit`} loading={working} onClick={submit}>
          {working ? t('Restoring…') : moving?.moved ? t('Try again') : t('Restore')}
        </Button>
        {!working && !moving && aside}
      </div>
    </div>
  )
}
