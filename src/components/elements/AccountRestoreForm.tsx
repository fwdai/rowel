import { useTranslation } from 'react-i18next'
import type { RemotePack } from '@/api/setup'
import AccountVaults from './AccountVaults'
import Button from './Button'
import RestoreForm from './RestoreForm'

interface Props {
  files: RemotePack[]
  selectedId: string | null
  onSelect: (id: string) => void
  /** A restore is in flight; every control stands down. */
  busy: boolean
  /** Restore the picked vault (see `RestoreForm`). */
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
 * The restore form for a vault in an account, whichever account it came from:
 * the account's vaults to pick from above the fields, and a wrong password
 * pointed at the one used on the user's other devices, which is where the
 * vault was sealed.
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

  return (
    <RestoreForm
      target={selectedId}
      busy={busy}
      onRestore={onRestore}
      wrongPassword={t(
        "That isn't the password this was sealed with. Try the one you use on your other devices."
      )}
      aside={
        onSwitchAccount && (
          <Button variant="pale" testid={`${testid}-switch-account`} onClick={onSwitchAccount}>
            {t('Switch account')}
          </Button>
        )
      }
      testid={testid}
    >
      <AccountVaults
        files={files}
        selectedId={selectedId}
        onSelect={onSelect}
        testid={pickerTestid}
      />
    </RestoreForm>
  )
}
