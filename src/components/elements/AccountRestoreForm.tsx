import { useState, type ChangeEvent } from 'react'
import { useTranslation } from 'react-i18next'
import type { RemotePack } from '@/api/setup'
import AccountVaults from './AccountVaults'
import Masterpass from './Masterpass'
import Button from './Button'
import { inputClass } from './formStyles'
import { unsealError } from '@/components/Start/shared/errors'

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
 * Name, password, Restore: the last step of every "make a workspace out of a
 * vault in an account" flow, whichever account the vault came from. The list and
 * the pick are the caller's — they outlive a wrong password — so this only
 * draws them.
 *
 * The name is optional: a vault carries its own, given wherever it was made,
 * and most restores want exactly that one. Typing one renames it everywhere.
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
  const [name, setName] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)

  const change = (event: ChangeEvent<HTMLInputElement>) => {
    setError(null)
    setPassword(event.currentTarget.value)
  }

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

  return (
    <div className="flex max-w-xs flex-col gap-3">
      <AccountVaults
        files={files}
        selectedId={selectedId}
        onSelect={onSelect}
        testid={pickerTestid}
      />
      <input
        type="text"
        className={inputClass}
        data-testid={`${testid}-name`}
        placeholder={t('Workspace name (optional)')}
        value={name}
        disabled={busy}
        onChange={event => setName(event.target.value)}
      />
      <Masterpass
        placeholder={t('Master password')}
        testid={`${testid}-password`}
        autoFocus={false}
        disabled={busy}
        error={error}
        onEnter={submit}
        onChange={change}
      />
      <div className="flex items-center gap-4">
        <Button size="md" testid={`${testid}-submit`} loading={busy} onClick={submit}>
          {busy ? t('Restoring…') : t('Restore')}
        </Button>
        {!busy && onSwitchAccount && (
          <Button
            variant="pale"
            size="md"
            testid={`${testid}-switch-account`}
            onClick={onSwitchAccount}
          >
            {t('Switch account')}
          </Button>
        )}
      </div>
    </div>
  )
}
