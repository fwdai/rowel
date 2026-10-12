import { useEffect } from 'react'
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
import SettingsRow from '@/components/elements/SettingsRow'
import Button from '@/components/elements/Button'
import AccountRestoreForm from '@/components/elements/AccountRestoreForm'
import ProviderButtons from '@/components/elements/ProviderButtons'
import ProviderMark from '@/components/elements/ProviderMark'
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

  // Leaving is backing out. A sign-in the user walked away from must never be
  // adopted by a later create or restore, so the pending tokens go with the
  // screen that asked for them — whether it goes because the user pressed
  // Cancel, moved to another section, or closed Settings. Harmless after a
  // restore that worked: the backend has already taken them. And harmless
  // during one: the backend holds the account for the restore's length and
  // refuses to drop it, so leaving mid-restore lets what the user asked for
  // finish — exactly as leaving mid-create does.
  useEffect(() => forgetAccount, [])

  const restoring = account.status === 'restoring'

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
      description={t(
        'Sign in and pick one of the vaults the account already holds. It arrives as a workspace of its own, unlocked with its own master password and syncing to the same pack.'
      )}
      testid="workspace-restore-row"
      control={
        account.status === 'idle' ? (
          <ProviderButtons
            variant="primary"
            testid="workspace-restore"
            onPick={connectWorkspaceAccount}
          />
        ) : restoring ? null : (
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
        <p data-testid="workspace-restore-waiting" className="text-sm text-text2">
          {name ? t('Waiting for {{provider}}…', name) : t('Waiting for the browser…')}
        </p>
      )}
      {account.status === 'empty' && (
        <p data-testid="workspace-restore-empty" className="text-sm text-text2">
          {name
            ? t('This {{provider}} account has nothing to restore', name)
            : t('This account has nothing to restore')}
        </p>
      )}
      {account.status === 'error' && (
        <p data-testid="workspace-restore-error" className="text-sm text-bad">
          {account.error || t('Something went wrong')}
        </p>
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
