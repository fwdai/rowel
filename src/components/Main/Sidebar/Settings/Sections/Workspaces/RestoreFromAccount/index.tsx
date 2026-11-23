import { useTranslation } from 'react-i18next'
import {
  useApp,
  connectWorkspaceAccount,
  forgetAccount,
  restoreWorkspaceFromPending,
  setupAccountSelect,
  switchWorkspaceAccount
} from '@/store'
import { providerName } from '@/api/provider'
import { useForgetAccountOnLeave } from '@/hooks/useForgetAccountOnLeave'
import SettingsRow from '@/components/elements/SettingsRow'
import Button from '@/components/elements/Button'
import AccountRestoreForm from '@/components/elements/AccountRestoreForm'
import Note from '@/components/elements/Note'
import ProviderButtons from '@/components/elements/ProviderButtons'
import ProviderMark from '@/components/elements/ProviderMark'
import { waitingFor } from '@/components/elements/providerCopy'
import { CloudGlyph } from '@/components/Main/icons'

// Adding a workspace that already exists somewhere else.
//
// A second device can restore exactly one vault, during onboarding; every other
// vault in the same account was unreachable from it. This is the way in to
// those: the user picks a provider, the account is connected while a vault is
// open here, its tokens are held in memory, and the vault the user picks
// arrives as a workspace of its own — already syncing, since the account is
// sealed under its key on the way.
//
// Everything on screen follows the probe in the store, exactly as the first
// run's account screen does, and for the same reason: the answer arrives as an
// event, so a result that lands while the user is reading simply redraws this.
export default function RestoreFromAccount() {
  const { t } = useTranslation()
  const account = useApp(state => state.setupAccount)
  const name = account.provider && { provider: providerName(account.provider) }

  // Leaving is backing out, as Cancel is.
  useForgetAccountOnLeave()

  const restoring = account.status === 'restoring'

  // The page's title and description say what this is, so the choice of a
  // provider stands alone, stacked on the phone as the first run's is; once
  // one is picked, the row names it.
  if (account.status === 'idle')
    return (
      <div data-testid="workspace-restore-row" className="px-4 py-3.5">
        <ProviderButtons
          variant="primary"
          testid="workspace-restore"
          onPick={connectWorkspaceAccount}
          className="flex flex-wrap gap-2 max-md:flex-col"
        />
      </div>
    )

  return (
    <SettingsRow
      label={name ? t('Restore from {{provider}}', name) : t('Restore from a cloud account')}
      icon={
        account.provider ? (
          <ProviderMark provider={account.provider} size={16} />
        ) : (
          <CloudGlyph size={16} />
        )
      }
      testid="workspace-restore-row"
      control={
        restoring ? null : (
          // Withdrawn while the restore runs, with the form's own Switch
          // account: the account is spoken for, and a Cancel that the backend
          // would refuse is a lie.
          <Button
            variant="pale"
            size="md"
            testid="workspace-restore-cancel"
            onClick={forgetAccount}
          >
            {t('Cancel')}
          </Button>
        )
      }
    >
      {account.status === 'pending' && (
        <Note testid="workspace-restore-waiting">{waitingFor(t, account.provider)}</Note>
      )}
      {account.status === 'empty' && (
        <Note testid="workspace-restore-empty">
          {name
            ? t('This {{provider}} account has nothing to restore', name)
            : t('This account has nothing to restore')}
        </Note>
      )}
      {account.status === 'error' && (
        <Note tone="bad" testid="workspace-restore-error">
          {account.error || t('Something went wrong')}
        </Note>
      )}
      {(account.status === 'found' || restoring) && (
        <AccountRestoreForm
          files={account.files}
          selectedId={account.selectedId}
          onSelect={setupAccountSelect}
          busy={restoring}
          onRestore={restoreWorkspaceFromPending}
          onSwitchAccount={switchWorkspaceAccount}
        />
      )}
    </SettingsRow>
  )
}
