import { useId, useState, type KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import type { SetupDriveFile } from '@/api/setup'
import { cx } from '@/utils/cx'
import DriveVaults from './DriveVaults'
import Button from './Button'
import Field from './Field'
import Note from './Note'
import { inputClass } from './formStyles'
import { verbatimInput } from './inputProps'
import { unsealError } from '@/components/Start/shared/errors'

interface Props {
  files: SetupDriveFile[]
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
 * Name, password, Restore: the last step of every "make a workspace out of a
 * vault on Drive" flow, whichever account the vault came from. The list and
 * the pick are the caller's — they outlive a wrong password — so this only
 * draws them.
 *
 * The name is optional: a vault carries its own, given wherever it was made,
 * and most restores want exactly that one. Typing one renames it everywhere.
 *
 * Plain labelled fields, like every other form in Settings: the lock screen's
 * card is for the lock screen.
 */
export default function DriveRestoreForm({
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

  const submit = () => {
    if (busy || !selectedId) return
    setError(null)
    onRestore(name.trim(), password, selectedId).catch((err: unknown) => {
      setError(
        unsealError(
          t,
          err,
          t("That isn't the password this was sealed with. Try the one you use on your other devices.")
        )
      )
    })
  }

  // Not while an IME is composing: that Enter confirms the characters, and a
  // restore started on it would carry a half-typed name across devices.
  const onEnter = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter' && !event.nativeEvent.isComposing) submit()
  }

  return (
    <div className="flex max-w-sm flex-col gap-3">
      <DriveVaults files={files} selectedId={selectedId} onSelect={onSelect} testid={pickerTestid} />
      <Field id={`${id}-name`} label={t('Workspace name (optional)')}>
        <input
          id={`${id}-name`}
          type="text"
          className={inputClass}
          data-testid={`${testid}-name`}
          value={name}
          disabled={busy}
          onChange={event => setName(event.target.value)}
          onKeyDown={onEnter}
        />
      </Field>
      <Field id={`${id}-password`} label={t('Master password')}>
        <input
          id={`${id}-password`}
          type="password"
          {...verbatimInput}
          className={cx(inputClass, error && 'border-bad')}
          data-testid={`${testid}-password`}
          aria-invalid={!!error}
          value={password}
          disabled={busy}
          onChange={event => {
            setError(null)
            setPassword(event.target.value)
          }}
          onKeyDown={onEnter}
        />
        {error && (
          <Note tone="bad" testid="form-error">
            {error}
          </Note>
        )}
      </Field>
      <div className="flex items-center gap-3">
        <Button testid={`${testid}-submit`} loading={busy} onClick={submit}>
          {busy ? t('Restoring…') : t('Restore')}
        </Button>
        {!busy && onSwitchAccount && (
          <Button variant="pale" testid={`${testid}-switch-account`} onClick={onSwitchAccount}>
            {t('Switch account')}
          </Button>
        )}
      </div>
    </div>
  )
}
