import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  useApp,
  useVault,
  selectWorkspaces,
  selectActiveWorkspace,
  switchWorkspace,
  openPasswordPrompt
} from '@/store'
import { messageOf } from '@/api/errors'
import type { Workspace } from '@/api/types'
import {
  syncDeletedRemotely,
  syncErrorText,
  syncPasswordChangedRemotely,
  syncReplaceRemote,
  syncReupload,
  syncUnderRetiredPassword
} from '@/api/sync'
import Button from '@/components/elements/Button'
import ConfirmSheet from '@/components/elements/ConfirmSheet'
import Note from '@/components/elements/Note'
import { useSubpage } from '../../sectionNav'
import WorkspaceRow from './WorkspaceRow'
import SyncProblem from './SyncProblem'
import { workspaceAbout, workspaceProvider } from './about'

const REPLACE_TITLE_ID = 'workspace-replace-remote-title'

export default function WorkspaceList() {
  const { t } = useTranslation()
  const list = useApp(selectWorkspaces)
  const active = useApp(selectActiveWorkspace)
  const sync = useApp(state => state.sync)
  const liveCount = useVault(state => state.items.length)
  // The one way a switch is refused: a sync flow is mid-flight in this
  // workspace, and moving the paths under it would land its files elsewhere.
  const [error, setError] = useState<string | null>(null)
  const { open } = useSubpage()
  // The confirmation is a page under this section, like Edit.
  const askDelete = (id: string) => open({ key: 'delete-workspace', id })

  // A device always has a vault to open, so the last workspace stays. Every
  // other one may go, the open one included: deleting it ends its session and
  // the app comes back on a survivor's lock screen.
  const last = list.length < 2

  // The account says *this* vault was deleted — from another device, or by
  // someone with access to the account — so its runs stop here. The copy here
  // is intact either way: the user can put it back in the account, or delete
  // it here too. The delete offered is the local one: the account is not asked
  // to remove anything more.
  const strandedHere = syncDeletedRemotely(sync)
  const reupload = () => syncReupload().catch((err: unknown) => setError(messageOf(err)))

  // Another device changed this vault's master password: the account's copy
  // opens only under the new one, which the shared prompt asks for.
  const passwordChanged = syncPasswordChangedRemotely(sync)

  // The account's copy is under a password this device already replaced and
  // can no longer open. Overwriting it with this copy is the one way out from
  // here, and it loses whatever only that copy had — so it is confirmed first.
  const retired = syncUnderRetiredPassword(sync)
  const [confirmReplace, setConfirmReplace] = useState(false)
  const replace = () => {
    setConfirmReplace(false)
    syncReplaceRemote().catch((err: unknown) => setError(messageOf(err)))
  }

  // Where its copy lives, if it has one to delete along with it.
  const provider = (workspace: Workspace) => workspaceProvider(workspace, { active, sync })

  return (
    <>
      {list.map(workspace => {
        const current = workspace.id === active
        return (
          <WorkspaceRow
            key={workspace.id}
            workspace={workspace}
            current={current}
            last={last}
            about={workspaceAbout(
              workspace,
              { current, provider: provider(workspace), liveCount },
              t
            )}
            onSwitch={() =>
              switchWorkspace(workspace.id).catch((err: unknown) => setError(messageOf(err)))
            }
            onDelete={() => askDelete(workspace.id)}
          />
        )
      })}
      {strandedHere && (
        <SyncProblem testid="workspace-deleted-remotely" text={syncErrorText(sync)}>
          <Button variant="pale" size="md" testid="workspace-reupload-stranded" onClick={reupload}>
            {t('Re-upload it')}
          </Button>
          <Button
            variant="pale-danger"
            size="md"
            testid="workspace-delete-stranded"
            disabled={last}
            onClick={() => askDelete(active)}
          >
            {t('Delete it here too')}
          </Button>
        </SyncProblem>
      )}
      {passwordChanged && (
        <SyncProblem testid="workspace-password-changed" text={syncErrorText(sync)}>
          <Button
            variant="pale"
            size="md"
            testid="workspace-adopt-password"
            onClick={openPasswordPrompt}
          >
            {t('Enter new password')}
          </Button>
        </SyncProblem>
      )}
      {retired && (
        <SyncProblem testid="workspace-retired-password" text={syncErrorText(sync)}>
          <Button
            variant="pale-danger"
            size="md"
            testid="workspace-replace-remote"
            onClick={() => setConfirmReplace(true)}
          >
            {t("Replace the account's copy with this device's")}
          </Button>
        </SyncProblem>
      )}
      {confirmReplace && (
        <ConfirmSheet
          id={REPLACE_TITLE_ID}
          testid="workspace-replace-remote-sheet"
          actionTestid="workspace-replace-remote-confirm"
          title={t("Replace the account's copy with this device's")}
          message={t(
            'Edits made only on the other device will be lost. The other device will be asked for the new password.'
          )}
          action={t('Replace')}
          onConfirm={replace}
          onClose={() => setConfirmReplace(false)}
        />
      )}
      {last && (
        <Note testid="workspace-delete-last" className="px-4 py-3 inset-shadow-hairline">
          {t('Your only lockbox cannot be deleted. Add another one first.')}
        </Note>
      )}
      {error && (
        <Note tone="bad" testid="workspace-switch-error" className="px-4 py-3 inset-shadow-hairline">
          {error}
        </Note>
      )}
    </>
  )
}
