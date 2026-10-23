import { useId, useState, type KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import type { RemotePack } from '@/api/setup'
import { describeError, vaultPasswordRequiredFor } from '@/api/errors'
import { moveRestoredLockbox, switchWorkspace } from '@/store'
import AccountVaults from '../AccountVaults'
import Button from '../Button'
import Field from '../Field'
import Note from '../Note'
import { inputClass } from '../formStyles'
import { unsealError } from '@/components/Start/shared/errors'
import SecretField from './SecretField'

interface Props {
  files: RemotePack[]
  selectedId: string | null
  onSelect: (id: string) => void
  /** A restore is in flight; every control stands down. */
  busy: boolean
  /**
   * Restore the picked vault as a workspace called `name`, unlocked with
   * `password`. An empty `name` leaves the naming to the backend, which takes
   * the one the vault carries in its pack. A rejection is shown under the
   * password field — a wrong one as the same sentence every unseal uses.
   */
  onRestore: (name: string, password: string, fileId: string) => Promise<void>
  /**
   * Sign in as somebody else. Offered only where the account can change: a
   * probe's pending account can, the open workspace's own cannot.
   */
  onSwitchAccount?: () => void
  /** Test id base for the form's controls; the picker's is `pickerTestid`. */
  testid?: string
  pickerTestid?: string
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
 * vault in an account" flow, whichever account the vault came from. The list and
 * the pick are the caller's — they outlive a wrong password — so this only
 * draws them.
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
export default function AccountRestoreForm({
  files,
  selectedId,
  onSelect,
  busy,
  onRestore,
  onSwitchAccount,
  testid = 'workspace-restore',
  pickerTestid = 'drive'
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
  const [picked, setPicked] = useState(selectedId)
  if (picked !== selectedId) {
    setPicked(selectedId)
    setMoving(null)
    setVaultPassword('')
    setVaultError(null)
    setOpenError(null)
  }

  const restore = () => {
    if (busy || !selectedId) return
    setError(null)
    const typed = password
    onRestore(name.trim(), typed, selectedId).catch((err: unknown) => {
      const workspaceId = vaultPasswordRequiredFor(err)
      if (workspaceId) {
        setMoving({ workspaceId, moved: false, lockboxPassword: typed })
        return
      }
      setError(
        unsealError(
          t,
          err,
          t("That isn't the password this was sealed with. Try the one you use on your other devices.")
        )
      )
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
      <AccountVaults
        files={files}
        selectedId={selectedId}
        onSelect={onSelect}
        testid={pickerTestid}
      />
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
        {!working && !moving && onSwitchAccount && (
          <Button variant="pale" testid={`${testid}-switch-account`} onClick={onSwitchAccount}>
            {t('Switch account')}
          </Button>
        )}
      </div>
    </div>
  )
}
